//! Importers for general Japanese dictionary sources.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::Result;

use crate::dict::{DictionaryCategory, DictionarySource};
use crate::dictionary_source::{NormalizedDictionaryEntry, merge_normalized_entries};

fn category_from_labels(labels: &str) -> DictionaryCategory {
    if labels.contains("成句")
        || labels.contains("慣用")
        || labels.contains("&exp;")
        || labels.contains("&id;")
    {
        DictionaryCategory::Idiom
    } else if labels.contains("人名") {
        DictionaryCategory::Person
    } else if labels.contains("組織") {
        DictionaryCategory::Organization
    } else if labels.contains("地名") {
        DictionaryCategory::Place
    } else {
        DictionaryCategory::General
    }
}

/// Import Mozc/user-dictionary TSV (`reading`, `surface`, POS, comment).
pub fn import_mozc(path: impl AsRef<Path>) -> Result<Vec<NormalizedDictionaryEntry>> {
    let reader = BufReader::new(File::open(path.as_ref())?);
    let mut entries = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 2 {
            continue;
        }
        let labels = columns.get(2).copied().unwrap_or_default();
        let description = columns
            .get(3)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(entry) = NormalizedDictionaryEntry::new(
            columns[0],
            columns[1],
            index as f32,
            DictionarySource::Mozc,
            category_from_labels(labels),
            description,
        ) {
            entries.push(entry);
        }
    }
    Ok(merge_normalized_entries(entries))
}

/// Import Sudachi CSV. The system cost is retained as the dictionary score.
pub fn import_sudachi(path: impl AsRef<Path>) -> Result<Vec<NormalizedDictionaryEntry>> {
    let reader = BufReader::new(File::open(path.as_ref())?);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let columns: Vec<&str> = line.trim().split(',').collect();
        if columns.len() < 12 {
            continue;
        }
        let Ok(cost) = columns[3].parse::<f32>() else {
            continue;
        };
        let labels = columns.get(5..8).unwrap_or_default().join("/");
        if let Some(entry) = NormalizedDictionaryEntry::new(
            columns[11],
            columns[4],
            cost,
            DictionarySource::Sudachi,
            category_from_labels(&labels),
            None,
        ) {
            entries.push(entry);
        }
    }
    Ok(merge_normalized_entries(entries))
}

fn extract_xml_tags(block: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut values = Vec::new();
    let mut remaining = block;
    while let Some(start) = remaining.find(&open) {
        let content = &remaining[start + open.len()..];
        let Some(end) = content.find(&close) else {
            break;
        };
        values.push(
            content[..end]
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&apos;", "'"),
        );
        remaining = &content[end + close.len()..];
    }
    values
}

fn jmdict_score(block: &str) -> f32 {
    let priority = extract_xml_tags(block, "ke_pri")
        .into_iter()
        .chain(extract_xml_tags(block, "re_pri"))
        .collect::<Vec<_>>();
    let mut score = if priority
        .iter()
        .any(|value| matches!(value.as_str(), "news1" | "ichi1" | "spec1" | "gai1"))
    {
        0.0
    } else if priority.iter().any(|value| value.ends_with('2')) {
        100.0
    } else {
        500.0
    };
    if block.contains("&arch;") || block.contains("&obsc;") || block.contains("&rare;") {
        score += 1000.0;
    }
    score
}

fn import_jmdict_entry(block: &str, output: &mut Vec<NormalizedDictionaryEntry>) {
    let readings = extract_xml_tags(block, "reb");
    let mut surfaces = extract_xml_tags(block, "keb");
    if surfaces.is_empty() {
        surfaces = readings.clone();
    }
    let category = category_from_labels(block);
    let score = jmdict_score(block);
    for reading in &readings {
        for surface in &surfaces {
            if let Some(entry) = NormalizedDictionaryEntry::new(
                reading,
                surface,
                score,
                DictionarySource::JMdict,
                category,
                (category == DictionaryCategory::Idiom).then(|| "慣用句".to_string()),
            ) {
                output.push(entry);
            }
        }
    }
}

/// Stream-import JMdict XML without retaining the complete XML document.
pub fn import_jmdict(path: impl AsRef<Path>) -> Result<Vec<NormalizedDictionaryEntry>> {
    let reader = BufReader::new(File::open(path.as_ref())?);
    let mut entries = Vec::new();
    let mut block = String::new();
    let mut inside_entry = false;
    for line in reader.lines() {
        let line = line?;
        if line.contains("<entry>") {
            inside_entry = true;
            block.clear();
        }
        if inside_entry {
            block.push_str(&line);
            block.push('\n');
        }
        if inside_entry && line.contains("</entry>") {
            import_jmdict_entry(&block, &mut entries);
            inside_entry = false;
        }
    }
    Ok(merge_normalized_entries(entries))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn imports_common_idiom_from_all_general_sources() {
        let mut mozc = tempfile::NamedTempFile::new().unwrap();
        writeln!(mozc, "じゅっちゅうはっく\t十中八九\t慣用句\t").unwrap();
        let mozc_entries = import_mozc(mozc.path()).unwrap();
        assert_eq!(mozc_entries[0].category, DictionaryCategory::Idiom);

        let mut sudachi = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            sudachi,
            "0,1,1,200,十中八九,名詞,普通名詞,成句,*,*,*,ジュッチュウハック"
        )
        .unwrap();
        let sudachi_entries = import_sudachi(sudachi.path()).unwrap();
        assert_eq!(sudachi_entries[0].reading, "じゅっちゅうはっく");
        assert_eq!(sudachi_entries[0].surface, "十中八九");

        let mut jmdict = tempfile::NamedTempFile::new().unwrap();
        write!(
            jmdict,
            "<JMdict><entry><k_ele><keb>十中八九</keb><ke_pri>ichi1</ke_pri></k_ele>\
             <r_ele><reb>じっちゅうはっく</reb></r_ele><sense><pos>&exp;</pos></sense>\
             </entry></JMdict>"
        )
        .unwrap();
        let jmdict_entries = import_jmdict(jmdict.path()).unwrap();
        assert_eq!(jmdict_entries[0].reading, "じっちゅうはっく");
        assert_eq!(jmdict_entries[0].score, 0.0);
        assert_eq!(jmdict_entries[0].category, DictionaryCategory::Idiom);

        let dictionary = crate::Dictionary::build_from_normalized(
            mozc_entries
                .into_iter()
                .chain(sudachi_entries)
                .chain(jmdict_entries),
        )
        .unwrap();
        for reading in ["じゅっちゅうはっく", "じっちゅうはっく"] {
            assert!(
                dictionary
                    .exact_match_search(reading)
                    .unwrap()
                    .candidates
                    .iter()
                    .any(|candidate| candidate.surface == "十中八九")
            );
        }
    }

    #[test]
    fn jmdict_rare_entries_rank_below_priority_entries() {
        assert!(jmdict_score("<ke_pri>ichi1</ke_pri>") < jmdict_score("<misc>&rare;</misc>"));
    }
}
