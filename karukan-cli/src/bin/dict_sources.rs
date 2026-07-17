//! Verify and fetch version-pinned dictionary source archives.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use karukan_engine::dictionary_source::{DictionarySourceSpec, DictionarySourcesManifest};
use sha2::{Digest, Sha256};

#[derive(Debug, Parser)]
#[command(name = "karukan-dict-sources")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate source metadata without downloading anything.
    Verify {
        #[arg(default_value = "dictionary-sources.toml")]
        manifest: PathBuf,
    },
    /// Fetch every source and verify its SHA-256.
    Fetch {
        #[arg(default_value = "dictionary-sources.toml")]
        manifest: PathBuf,
        #[arg(long, default_value = "target/dictionary-sources")]
        cache_dir: PathBuf,
    },
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn verify_bytes(source: &DictionarySourceSpec, bytes: &[u8]) -> Result<()> {
    let actual = sha256(bytes);
    if !actual.eq_ignore_ascii_case(&source.sha256) {
        bail!(
            "SHA-256 mismatch for '{}': expected {}, got {}",
            source.name,
            source.sha256,
            actual
        );
    }
    Ok(())
}

async fn fetch_source(source: &DictionarySourceSpec, cache_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(cache_dir)?;
    let output = cache_dir.join(&source.filename);
    if output.exists() {
        let bytes = std::fs::read(&output)?;
        if verify_bytes(source, &bytes).is_ok() {
            return Ok(output);
        }
    }

    let bytes = if let Some(path) = source.url.strip_prefix("file://") {
        std::fs::read(path).with_context(|| format!("failed to read local source {path}"))?
    } else {
        reqwest::get(&source.url)
            .await
            .with_context(|| format!("failed to download {}", source.url))?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec()
    };
    verify_bytes(source, &bytes)?;
    std::fs::write(&output, bytes)?;
    Ok(output)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Verify { manifest } => {
            let manifest = DictionarySourcesManifest::load(&manifest)?;
            eprintln!("{} source(s) verified", manifest.sources.len());
        }
        Command::Fetch {
            manifest,
            cache_dir,
        } => {
            let manifest = DictionarySourcesManifest::load(&manifest)?;
            for source in &manifest.sources {
                let path = fetch_source(source, &cache_dir).await?;
                eprintln!(
                    "{} {} ready at {}",
                    source.name,
                    source.version,
                    path.display()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use karukan_engine::dictionary_source::SourceArchive;

    fn source(hash: String) -> DictionarySourceSpec {
        DictionarySourceSpec {
            name: "fixture".to_string(),
            version: "1".to_string(),
            url: "file:///fixture".to_string(),
            sha256: hash,
            license: "CC0-1.0".to_string(),
            priority: 0,
            filename: "fixture".to_string(),
            archive: SourceArchive::None,
        }
    }

    #[test]
    fn verifies_fixture_sha256() {
        let bytes = b"karukan dictionary fixture\n";
        assert!(verify_bytes(&source(sha256(bytes)), bytes).is_ok());
        assert!(verify_bytes(&source("0".repeat(64)), bytes).is_err());
    }
}
