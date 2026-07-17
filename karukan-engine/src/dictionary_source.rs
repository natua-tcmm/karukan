//! Reproducible dictionary-source manifests and normalized interchange records.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::dict::{DictionaryCategory, DictionarySource};
use crate::kana::katakana_to_hiragana;

/// Top-level `dictionary-sources.toml` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionarySourcesManifest {
    pub schema_version: u32,
    #[serde(default)]
    pub sources: Vec<DictionarySourceSpec>,
}

/// One version-pinned external dictionary source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionarySourceSpec {
    pub name: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub license: String,
    pub priority: i32,
    pub filename: String,
    #[serde(default)]
    pub archive: SourceArchive,
}

/// Archive type recorded for the importer that consumes a fetched source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceArchive {
    #[default]
    None,
    Zip,
    TarGz,
    Gzip,
}

impl DictionarySourcesManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let manifest: Self = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "unsupported dictionary source schema: {}",
                self.schema_version
            );
        }
        let mut names = HashSet::new();
        for source in &self.sources {
            if !names.insert(source.name.as_str()) {
                bail!("duplicate dictionary source name: {}", source.name);
            }
            if source.name.trim().is_empty()
                || source.version.trim().is_empty()
                || source.url.trim().is_empty()
                || source.filename.trim().is_empty()
                || source.license.trim().is_empty()
            {
                bail!(
                    "dictionary source '{}' has an empty required field",
                    source.name
                );
            }
            if source.sha256.len() != 64
                || !source.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                bail!("dictionary source '{}' has an invalid SHA-256", source.name);
            }
        }
        Ok(())
    }
}

/// Common intermediate record emitted by every dictionary importer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedDictionaryEntry {
    pub reading: String,
    pub surface: String,
    pub score: f32,
    pub source: DictionarySource,
    pub category: DictionaryCategory,
    #[serde(default)]
    pub description: Option<String>,
}

impl NormalizedDictionaryEntry {
    pub fn new(
        reading: &str,
        surface: &str,
        score: f32,
        source: DictionarySource,
        category: DictionaryCategory,
        description: Option<String>,
    ) -> Option<Self> {
        let reading = normalize_reading(reading);
        let surface = normalize_surface(surface);
        if reading.is_empty() || surface.is_empty() {
            return None;
        }
        Some(Self {
            reading,
            surface,
            score,
            source,
            category,
            description: description
                .map(|value| normalize_surface(&value))
                .filter(|value| !value.is_empty()),
        })
    }
}

/// NFKC-normalize a reading and canonicalize katakana to hiragana.
pub fn normalize_reading(reading: &str) -> String {
    let normalized: String = reading.trim().nfkc().collect();
    katakana_to_hiragana(&normalized)
}

/// NFKC-normalize and trim a surface or description.
pub fn normalize_surface(surface: &str) -> String {
    surface.trim().nfkc().collect()
}

/// Merge duplicate `(reading, surface)` pairs deterministically.
///
/// Lower scores rank better in the current dictionary format. On a tie, the
/// first record wins so source priority can be expressed by input order.
pub fn merge_normalized_entries(
    entries: impl IntoIterator<Item = NormalizedDictionaryEntry>,
) -> Vec<NormalizedDictionaryEntry> {
    let mut merged: BTreeMap<(String, String), NormalizedDictionaryEntry> = BTreeMap::new();
    for entry in entries {
        let key = (entry.reading.clone(), entry.surface.clone());
        match merged.get_mut(&key) {
            Some(existing) if entry.score < existing.score => *existing = entry,
            Some(_) => {}
            None => {
                merged.insert(key, entry);
            }
        }
    }
    merged.into_values().collect()
}

pub fn write_jsonl(path: impl AsRef<Path>, entries: &[NormalizedDictionaryEntry]) -> Result<()> {
    let file = File::create(path.as_ref())?;
    let mut writer = BufWriter::new(file);
    for entry in entries {
        serde_json::to_writer(&mut writer, entry)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

pub fn read_jsonl(path: impl AsRef<Path>) -> Result<Vec<NormalizedDictionaryEntry>> {
    let file = File::open(path.as_ref())?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        entries.push(
            serde_json::from_str(&line)
                .with_context(|| format!("invalid JSONL record at line {}", index + 1))?,
        );
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_validation_rejects_duplicate_names() {
        let source = DictionarySourceSpec {
            name: "fixture".to_string(),
            version: "1".to_string(),
            url: "file:///fixture.tsv".to_string(),
            sha256: "0".repeat(64),
            license: "CC0-1.0".to_string(),
            priority: 10,
            filename: "fixture.tsv".to_string(),
            archive: SourceArchive::None,
        };
        let manifest = DictionarySourcesManifest {
            schema_version: 1,
            sources: vec![source.clone(), source],
        };
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn normalizes_and_merges_fixture_records() {
        let entries = vec![
            NormalizedDictionaryEntry::new(
                "ジュッチュウハック",
                "十中八九",
                10.0,
                DictionarySource::Sudachi,
                DictionaryCategory::Idiom,
                None,
            )
            .unwrap(),
            NormalizedDictionaryEntry::new(
                "じゅっちゅうはっく",
                "十中八九",
                5.0,
                DictionarySource::JMdict,
                DictionaryCategory::Idiom,
                Some("慣用句".to_string()),
            )
            .unwrap(),
        ];
        let merged = merge_normalized_entries(entries);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].reading, "じゅっちゅうはっく");
        assert_eq!(merged[0].score, 5.0);
        assert_eq!(merged[0].description.as_deref(), Some("慣用句"));
    }

    #[test]
    fn jsonl_round_trip() {
        let entry = NormalizedDictionaryEntry::new(
            "トウキョウ",
            "東京都",
            1.0,
            DictionarySource::JapanPost,
            DictionaryCategory::Address,
            Some("東京都".to_string()),
        )
        .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        write_jsonl(file.path(), std::slice::from_ref(&entry)).unwrap();
        assert_eq!(read_jsonl(file.path()).unwrap(), vec![entry]);
    }
}
