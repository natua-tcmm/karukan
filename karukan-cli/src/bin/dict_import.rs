//! Import general vocabulary sources into normalized JSONL and KRKN v2.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use karukan_engine::Dictionary;
use karukan_engine::dictionary_import::{import_jmdict, import_mozc, import_sudachi};
use karukan_engine::dictionary_source::{merge_normalized_entries, write_jsonl};

#[derive(Debug, Parser)]
#[command(name = "karukan-dict-import")]
struct Cli {
    #[arg(long)]
    mozc: Vec<PathBuf>,
    #[arg(long)]
    sudachi: Vec<PathBuf>,
    #[arg(long)]
    jmdict: Vec<PathBuf>,
    #[arg(short, long, default_value = "general-dictionary.jsonl")]
    output: PathBuf,
    #[arg(long)]
    binary: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut entries = Vec::new();
    for path in &cli.mozc {
        entries.extend(import_mozc(path)?);
    }
    for path in &cli.sudachi {
        entries.extend(import_sudachi(path)?);
    }
    for path in &cli.jmdict {
        entries.extend(import_jmdict(path)?);
    }
    let entries = merge_normalized_entries(entries);
    write_jsonl(&cli.output, &entries)?;
    eprintln!(
        "{} normalized general-vocabulary records written to {}",
        entries.len(),
        cli.output.display()
    );
    if let Some(path) = cli.binary {
        Dictionary::build_from_normalized(entries)?.save(&path)?;
        eprintln!("KRKN v2 dictionary written to {}", path.display());
    }
    Ok(())
}
