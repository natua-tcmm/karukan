//! Import Japanese address and place-name sources into normalized JSONL/KRKN v2.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use karukan_engine::Dictionary;
use karukan_engine::dictionary_source::{merge_normalized_entries, write_jsonl};
use karukan_engine::geographic_import::{
    category_counts, import_gsi_places, import_japan_post, import_jmnedict, import_skk_places,
};

#[derive(Debug, Parser)]
#[command(name = "karukan-dict-geography")]
struct Cli {
    #[arg(long)]
    japan_post: Vec<PathBuf>,
    #[arg(long)]
    jmnedict: Vec<PathBuf>,
    #[arg(long)]
    gsi: Vec<PathBuf>,
    #[arg(long)]
    skk: Vec<PathBuf>,
    #[arg(short, long, default_value = "geographic-dictionary.jsonl")]
    output: PathBuf,
    #[arg(long)]
    binary: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut entries = Vec::new();
    for path in &cli.japan_post {
        entries.extend(import_japan_post(path)?);
    }
    for path in &cli.jmnedict {
        entries.extend(import_jmnedict(path)?);
    }
    for path in &cli.gsi {
        entries.extend(import_gsi_places(path)?);
    }
    for path in &cli.skk {
        entries.extend(import_skk_places(path)?);
    }
    let entries = merge_normalized_entries(entries);
    write_jsonl(&cli.output, &entries)?;
    eprintln!(
        "{} geographic records written to {}",
        entries.len(),
        cli.output.display()
    );
    let mut counts: Vec<_> = category_counts(&entries).into_iter().collect();
    counts.sort_by_key(|(category, _)| *category as u8);
    for (category, count) in counts {
        eprintln!("  {category:?}: {count}");
    }
    if let Some(path) = cli.binary {
        Dictionary::build_from_normalized(entries)?.save(&path)?;
        eprintln!("KRKN v2 dictionary written to {}", path.display());
    }
    Ok(())
}
