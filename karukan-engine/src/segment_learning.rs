//! Context-aware learning for explicitly corrected conversion segments.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnedSegment {
    pub reading: String,
    pub surface: String,
    pub left_hint: Option<String>,
    pub right_hint: Option<String>,
    pub frequency: u32,
    pub last_used: u64,
}

#[derive(Debug)]
pub struct SegmentLearningCache {
    entries: HashMap<String, Vec<LearnedSegment>>,
    max_entries: usize,
    dirty: bool,
}

impl SegmentLearningCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            dirty: false,
        }
    }

    pub fn record(
        &mut self,
        reading: &str,
        surface: &str,
        left_hint: Option<&str>,
        right_hint: Option<&str>,
    ) {
        let entries = self.entries.entry(reading.to_string()).or_default();
        if let Some(entry) = entries.iter_mut().find(|entry| {
            entry.surface == surface
                && entry.left_hint.as_deref() == left_hint
                && entry.right_hint.as_deref() == right_hint
        }) {
            entry.frequency = entry.frequency.saturating_add(1);
            entry.last_used = now_unix();
        } else {
            entries.push(LearnedSegment {
                reading: reading.to_string(),
                surface: surface.to_string(),
                left_hint: left_hint.map(str::to_string),
                right_hint: right_hint.map(str::to_string),
                frequency: 1,
                last_used: now_unix(),
            });
        }
        self.dirty = true;
        self.evict();
    }

    /// Matching context receives a strong bonus; context-free entries still
    /// remain usable with a smaller score.
    pub fn lookup(
        &self,
        reading: &str,
        left_hint: Option<&str>,
        right_hint: Option<&str>,
    ) -> Vec<(LearnedSegment, f64)> {
        let now = now_unix();
        let mut results = self
            .entries
            .get(reading)
            .into_iter()
            .flatten()
            .filter(|entry| {
                entry
                    .left_hint
                    .as_deref()
                    .map(|hint| Some(hint) == left_hint)
                    .unwrap_or(true)
                    && entry
                        .right_hint
                        .as_deref()
                        .map(|hint| Some(hint) == right_hint)
                        .unwrap_or(true)
            })
            .map(|entry| {
                let mut value = recency_frequency_score(entry, now);
                if entry.left_hint.is_some() && entry.left_hint.as_deref() == left_hint {
                    value += 20.0;
                }
                if entry.right_hint.is_some() && entry.right_hint.as_deref() == right_hint {
                    value += 30.0;
                }
                (entry.clone(), value)
            })
            .collect::<Vec<_>>();
        results.sort_by(|left, right| right.1.total_cmp(&left.1));
        results
    }

    pub fn load(path: &Path, max_entries: usize) -> anyhow::Result<Self> {
        let reader = std::io::BufReader::new(std::fs::File::open(path)?);
        let mut cache = Self::new(max_entries);
        for line in reader.lines() {
            let line = line?;
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let columns: Vec<_> = line.split('\t').collect();
            if columns.len() != 8 {
                continue;
            }
            let Ok(frequency) = columns[6].parse() else {
                continue;
            };
            let Ok(last_used) = columns[7].parse() else {
                continue;
            };
            let reading = unescape(columns[0]);
            cache
                .entries
                .entry(reading.clone())
                .or_default()
                .push(LearnedSegment {
                    reading,
                    surface: unescape(columns[1]),
                    left_hint: decode_hint(columns[2], columns[3]),
                    right_hint: decode_hint(columns[4], columns[5]),
                    frequency,
                    last_used,
                });
        }
        cache.dirty = false;
        Ok(cache)
    }

    pub fn save(&mut self, path: &Path) -> anyhow::Result<()> {
        self.evict();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut writer = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(writer, "# karukan segment learning cache v1")?;
        let mut readings: Vec<_> = self.entries.keys().collect();
        readings.sort();
        for reading in readings {
            if let Some(entries) = self.entries.get(reading) {
                for entry in entries {
                    let (left_present, left) = encode_hint(entry.left_hint.as_deref());
                    let (right_present, right) = encode_hint(entry.right_hint.as_deref());
                    writeln!(
                        writer,
                        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        escape(&entry.reading),
                        escape(&entry.surface),
                        left_present,
                        left,
                        right_present,
                        right,
                        entry.frequency,
                        entry.last_used
                    )?;
                }
            }
        }
        writer.flush()?;
        self.dirty = false;
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn entry_count(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }

    fn evict(&mut self) {
        while self.entry_count() > self.max_entries {
            let oldest = self
                .entries
                .iter()
                .flat_map(|(reading, entries)| {
                    entries
                        .iter()
                        .enumerate()
                        .map(move |(index, entry)| (reading.clone(), index, entry.last_used))
                })
                .min_by_key(|(_, _, last_used)| *last_used);
            let Some((reading, index, _)) = oldest else {
                break;
            };
            if let Some(entries) = self.entries.get_mut(&reading) {
                entries.remove(index);
                if entries.is_empty() {
                    self.entries.remove(&reading);
                }
            }
        }
    }
}

fn encode_hint(hint: Option<&str>) -> (&'static str, String) {
    match hint {
        Some(value) => ("1", escape(value)),
        None => ("0", String::new()),
    }
}

fn decode_hint(present: &str, encoded: &str) -> Option<String> {
    (present == "1").then(|| unescape(encoded))
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn unescape(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('t') => output.push('\t'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('\\') => output.push('\\'),
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }
    output
}

fn recency_frequency_score(entry: &LearnedSegment, now: u64) -> f64 {
    let age_days = now.saturating_sub(entry.last_used) / 86_400;
    10.0 / (1.0 + age_days as f64) + (entry.frequency as f64).ln_1p()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_context_promotes_hiragana_ato_before_comma() {
        let mut cache = SegmentLearningCache::new(100);
        cache.record("あと", "後", None, None);
        cache.record("あと", "あと", None, Some("、"));
        let results = cache.lookup("あと", None, Some("、"));
        assert_eq!(results[0].0.surface, "あと");
        assert_eq!(cache.lookup("あと", None, Some("で")).len(), 1);
        assert_eq!(cache.lookup("あと", None, Some("で"))[0].0.surface, "後");
    }

    #[test]
    fn tsv_round_trip_preserves_optional_hints() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut cache = SegmentLearningCache::new(100);
        cache.record("あと", "あと", Some("。"), Some("、"));
        cache.save(file.path()).unwrap();
        let loaded = SegmentLearningCache::load(file.path(), 100).unwrap();
        let entry = loaded.lookup("あと", Some("。"), Some("、"))[0].0.clone();
        assert_eq!(entry.left_hint.as_deref(), Some("。"));
        assert_eq!(entry.right_hint.as_deref(), Some("、"));
    }
}
