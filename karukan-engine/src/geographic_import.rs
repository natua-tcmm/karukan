//! Importers for Japanese addresses, place names, stations, and natural features.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::Result;

use crate::dict::{DictionaryCategory, DictionarySource};
use crate::dictionary_source::{NormalizedDictionaryEntry, merge_normalized_entries};

fn parse_csv_record(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(std::mem::take(&mut field));
            }
            _ => field.push(ch),
        }
    }
    fields.push(field);
    fields
}

fn cleaned_town(value: &str) -> Option<&str> {
    if value.contains("以下に掲載がない場合") {
        return None;
    }
    Some(value.split(['（', '(']).next().unwrap_or(value).trim()).filter(|value| !value.is_empty())
}

fn push_entry(
    output: &mut Vec<NormalizedDictionaryEntry>,
    reading: &str,
    surface: &str,
    score: f32,
    source: DictionarySource,
    category: DictionaryCategory,
    description: Option<String>,
) {
    if let Some(entry) =
        NormalizedDictionaryEntry::new(reading, surface, score, source, category, description)
    {
        output.push(entry);
    }
}

/// Import Japan Post's UTF-8 `KEN_ALL` address CSV.
///
/// In addition to prefecture, municipality, and town records, this emits
/// composite readings so a complete address can be resolved in one lookup.
pub fn import_japan_post(path: impl AsRef<Path>) -> Result<Vec<NormalizedDictionaryEntry>> {
    let reader = BufReader::new(File::open(path.as_ref())?);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let columns = parse_csv_record(&line?);
        if columns.len() < 9 {
            continue;
        }
        let (pref_reading, city_reading) = (&columns[3], &columns[4]);
        let (prefecture, city) = (&columns[6], &columns[7]);
        push_entry(
            &mut entries,
            pref_reading,
            prefecture,
            50.0,
            DictionarySource::JapanPost,
            DictionaryCategory::Address,
            None,
        );
        push_entry(
            &mut entries,
            city_reading,
            city,
            40.0,
            DictionarySource::JapanPost,
            DictionaryCategory::Address,
            Some(prefecture.clone()),
        );
        push_entry(
            &mut entries,
            &format!("{pref_reading}{city_reading}"),
            &format!("{prefecture}{city}"),
            20.0,
            DictionarySource::JapanPost,
            DictionaryCategory::Address,
            None,
        );

        let (Some(town_reading), Some(town)) =
            (cleaned_town(&columns[5]), cleaned_town(&columns[8]))
        else {
            continue;
        };
        let parent = format!("{prefecture} {city}");
        push_entry(
            &mut entries,
            town_reading,
            town,
            30.0,
            DictionarySource::JapanPost,
            DictionaryCategory::Address,
            Some(parent.clone()),
        );
        push_entry(
            &mut entries,
            &format!("{city_reading}{town_reading}"),
            &format!("{city}{town}"),
            15.0,
            DictionarySource::JapanPost,
            DictionaryCategory::Address,
            Some(prefecture.clone()),
        );
        push_entry(
            &mut entries,
            &format!("{pref_reading}{city_reading}{town_reading}"),
            &format!("{prefecture}{city}{town}"),
            10.0,
            DictionarySource::JapanPost,
            DictionaryCategory::Address,
            None,
        );
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
                .replace("&gt;", ">"),
        );
        remaining = &content[end + close.len()..];
    }
    values
}

fn jmnedict_category(block: &str) -> DictionaryCategory {
    if block.contains("&station;") {
        DictionaryCategory::Station
    } else if block.contains("&geog;") || block.contains("&mount;") || block.contains("&river;") {
        DictionaryCategory::NaturalFeature
    } else if block.contains("&organization;") || block.contains("&company;") {
        DictionaryCategory::Organization
    } else if block.contains("&surname;") || block.contains("&given;") || block.contains("&person;")
    {
        DictionaryCategory::Person
    } else {
        DictionaryCategory::Place
    }
}

fn import_jmnedict_entry(block: &str, output: &mut Vec<NormalizedDictionaryEntry>) {
    let readings = extract_xml_tags(block, "reb");
    let surfaces = extract_xml_tags(block, "keb");
    let description = extract_xml_tags(block, "trans_det").into_iter().next();
    let category = jmnedict_category(block);
    for reading in &readings {
        for surface in &surfaces {
            push_entry(
                output,
                reading,
                surface,
                100.0,
                DictionarySource::JMnedict,
                category,
                description.clone(),
            );
        }
    }
}

/// Stream-import JMnedict XML.
pub fn import_jmnedict(path: impl AsRef<Path>) -> Result<Vec<NormalizedDictionaryEntry>> {
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
            import_jmnedict_entry(&block, &mut entries);
            inside_entry = false;
        }
    }
    Ok(merge_normalized_entries(entries))
}

fn header_index(headers: &[String], aliases: &[&str]) -> Option<usize> {
    headers
        .iter()
        .position(|header| aliases.iter().any(|alias| header.trim() == *alias))
}

fn gsi_category(kind: &str, surface: &str) -> DictionaryCategory {
    if kind.contains('駅') || surface.ends_with('駅') {
        DictionaryCategory::Station
    } else if ["山", "河川", "湖", "沼", "岬", "島", "海峡", "滝"]
        .iter()
        .any(|label| kind.contains(label))
    {
        DictionaryCategory::NaturalFeature
    } else {
        DictionaryCategory::Place
    }
}

/// Import a GSI place-name CSV using common Japanese or English header names.
pub fn import_gsi_places(path: impl AsRef<Path>) -> Result<Vec<NormalizedDictionaryEntry>> {
    let mut lines = BufReader::new(File::open(path.as_ref())?).lines();
    let Some(header) = lines.next().transpose()? else {
        return Ok(Vec::new());
    };
    let headers = parse_csv_record(header.trim_start_matches('\u{feff}'));
    let reading_index = header_index(&headers, &["読み仮名", "よみ", "読み", "reading"]);
    let surface_index = header_index(&headers, &["地名", "名称", "name"]);
    let kind_index = header_index(&headers, &["種別", "分類", "type"]);
    let pref_index = header_index(&headers, &["都道府県", "都道府県名", "prefecture"]);
    let city_index = header_index(&headers, &["市区町村", "市区町村名", "municipality"]);
    let (Some(reading_index), Some(surface_index)) = (reading_index, surface_index) else {
        return Ok(Vec::new());
    };

    let mut entries = Vec::new();
    for line in lines {
        let columns = parse_csv_record(&line?);
        let Some(reading) = columns.get(reading_index) else {
            continue;
        };
        let Some(surface) = columns.get(surface_index) else {
            continue;
        };
        let kind = kind_index
            .and_then(|index| columns.get(index))
            .map(String::as_str)
            .unwrap_or_default();
        let description = [pref_index, city_index]
            .into_iter()
            .flatten()
            .filter_map(|index| columns.get(index))
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        push_entry(
            &mut entries,
            reading,
            surface,
            150.0,
            DictionarySource::Gsi,
            gsi_category(kind, surface),
            (!description.is_empty()).then_some(description),
        );
    }
    Ok(merge_normalized_entries(entries))
}

/// Import optional SKK dictionary records (`reading /surface;annotation/.../`).
pub fn import_skk_places(path: impl AsRef<Path>) -> Result<Vec<NormalizedDictionaryEntry>> {
    let reader = BufReader::new(File::open(path.as_ref())?);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let Some((reading, candidates)) = line.split_once(' ') else {
            continue;
        };
        for candidate in candidates.trim_matches('/').split('/') {
            let (surface, description) = candidate
                .split_once(';')
                .map(|(surface, annotation)| (surface, Some(annotation.to_string())))
                .unwrap_or((candidate, None));
            let category = gsi_category(description.as_deref().unwrap_or_default(), surface);
            push_entry(
                &mut entries,
                reading,
                surface,
                300.0,
                DictionarySource::Skk,
                category,
                description,
            );
        }
    }
    Ok(merge_normalized_entries(entries))
}

/// Count records by category for importer diagnostics.
pub fn category_counts(
    entries: &[NormalizedDictionaryEntry],
) -> HashMap<DictionaryCategory, usize> {
    let mut counts = HashMap::new();
    for entry in entries {
        *counts.entry(entry.category).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn imports_japan_post_components_and_complete_address() {
        let mut csv = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            csv,
            "\"13113\",\"150  \",\"1500041\",\"トウキョウト\",\"シブヤク\",\"ジンナン\",\"東京都\",\"渋谷区\",\"神南\""
        )
        .unwrap();
        let entries = import_japan_post(csv.path()).unwrap();
        let full = entries
            .iter()
            .find(|entry| entry.reading == "とうきょうとしぶやくじんなん")
            .unwrap();
        assert_eq!(full.surface, "東京都渋谷区神南");
        let town = entries
            .iter()
            .find(|entry| entry.reading == "じんなん")
            .unwrap();
        assert_eq!(town.description.as_deref(), Some("東京都 渋谷区"));
    }

    #[test]
    fn imports_jmnedict_station_and_description() {
        let mut xml = tempfile::NamedTempFile::new().unwrap();
        write!(
            xml,
            "<JMnedict><entry><k_ele><keb>東京駅</keb></k_ele>\
             <r_ele><reb>とうきょうえき</reb></r_ele><trans>\
             <name_type>&station;</name_type><trans_det>東京都千代田区</trans_det>\
             </trans></entry></JMnedict>"
        )
        .unwrap();
        let entries = import_jmnedict(xml.path()).unwrap();
        assert_eq!(entries[0].category, DictionaryCategory::Station);
        assert_eq!(entries[0].description.as_deref(), Some("東京都千代田区"));
    }

    #[test]
    fn imports_gsi_natural_feature_and_skk_station() {
        let mut gsi = tempfile::NamedTempFile::new().unwrap();
        writeln!(gsi, "読み仮名,名称,種別,都道府県,市区町村").unwrap();
        writeln!(gsi, "ふじさん,富士山,山,静岡県,富士宮市").unwrap();
        let entries = import_gsi_places(gsi.path()).unwrap();
        assert_eq!(entries[0].category, DictionaryCategory::NaturalFeature);
        assert_eq!(entries[0].description.as_deref(), Some("静岡県 富士宮市"));

        let mut skk = tempfile::NamedTempFile::new().unwrap();
        writeln!(skk, "しんじゅくえき /新宿駅;駅/").unwrap();
        let entries = import_skk_places(skk.path()).unwrap();
        assert_eq!(entries[0].category, DictionaryCategory::Station);
    }
}
