//! Settings configuration
//!
//! Manages user-configurable settings for the IME.
//! Default values are defined in `config/default.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Default configuration TOML embedded from config/default.toml
const DEFAULT_CONFIG_TOML: &str = include_str!("../../config/default.toml");

/// Configuration settings for the IME
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Conversion settings
    pub conversion: ConversionSettings,
    /// Learning cache settings
    pub learning: LearningSettings,
}

/// Conversion-related settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionSettings {
    /// Number of model candidates generated during live conversion
    pub live_num_candidates: usize,
    /// Number of candidates to show on Space conversion
    pub num_candidates: usize,
    /// Use surrounding text (text left of cursor) as context for conversion
    pub use_context: bool,
    /// Maximum number of surrounding text characters passed to the conversion API
    pub max_context_length: usize,
    /// Maximum reading length (in characters) converted by the model in a single
    /// call during live conversion. The composing buffer is split into chunks
    /// of at most this many characters so per-keystroke latency stays bounded
    /// for long input; each chunk's left context is the converted text of the
    /// preceding chunks.
    pub composing_chunk_len: usize,
    /// Path to dictionary binary file (optional, defaults to data_dir/dict.bin)
    pub dict_path: Option<String>,
    /// Model variant id (optional, defaults to registry default)
    pub model: Option<String>,
    /// Number of threads for llama.cpp inference (0 = all cores, llama.cpp default)
    pub n_threads: u32,
    /// Enable live conversion at startup
    pub live_conversion: bool,
}

/// Learning cache settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningSettings {
    /// Whether learning is enabled
    pub enabled: bool,
    /// Maximum number of total entries in the learning cache
    pub max_entries: usize,
}

impl Default for Settings {
    fn default() -> Self {
        toml::from_str(DEFAULT_CONFIG_TOML).expect("embedded default.toml must be valid")
    }
}

/// Recursively merge `overlay` TOML values on top of `base`.
fn merge_toml(base: &mut toml::Value, overlay: &toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, value) in overlay_table {
                if let Some(base_value) = base_table.get_mut(key) {
                    merge_toml(base_value, value);
                } else {
                    base_table.insert(key.clone(), value.clone());
                }
            }
        }
        (base, _) => {
            *base = overlay.clone();
        }
    }
}

const DEPRECATED_CONVERSION_KEYS: &[&str] = &[
    "strategy",
    "light_model",
    "short_input_threshold",
    "beam_width",
    "max_latency_ms",
];

fn warn_deprecated_conversion_keys(user: &toml::Value) {
    let Some(conversion) = user.get("conversion").and_then(toml::Value::as_table) else {
        return;
    };
    let deprecated: Vec<_> = DEPRECATED_CONVERSION_KEYS
        .iter()
        .copied()
        .filter(|key| conversion.contains_key(*key))
        .collect();
    if !deprecated.is_empty() {
        warn!(
            "Ignoring deprecated conversion settings: {}",
            deprecated.join(", ")
        );
    }
}

/// Parse user TOML content merged on top of default.toml.
fn parse_with_defaults(user_content: &str) -> Result<Settings> {
    let mut base: toml::Value = toml::from_str(DEFAULT_CONFIG_TOML)?;
    let user: toml::Value = toml::from_str(user_content)?;
    warn_deprecated_conversion_keys(&user);
    merge_toml(&mut base, &user);
    let settings: Settings = base.try_into()?;
    Ok(settings)
}

/// Get the project directories for karukan-im.
fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("com", "karukan", "karukan-im")
}

impl Settings {
    /// Get the data directory path
    pub fn data_dir() -> Option<PathBuf> {
        project_dirs().map(|dirs| dirs.data_dir().to_path_buf())
    }

    /// Get the configuration directory path
    pub fn config_dir() -> Option<PathBuf> {
        project_dirs().map(|dirs| dirs.config_dir().to_path_buf())
    }

    /// Get the configuration file path
    pub fn config_file() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("config.toml"))
    }

    /// Get the user dictionary directory path.
    ///
    /// All files in this directory are automatically loaded as user dictionaries.
    /// Default: `~/.local/share/karukan-im/user_dicts/`
    pub fn user_dict_dir() -> Option<PathBuf> {
        Self::data_dir().map(|dir| dir.join("user_dicts"))
    }

    /// Get the learning cache file path.
    ///
    /// Default: `~/.local/share/karukan-im/learning.tsv`
    pub fn learning_file() -> Option<PathBuf> {
        Self::data_dir().map(|dir| dir.join("learning.tsv"))
    }

    /// Get the context-aware segment learning cache file path.
    ///
    /// Default: `~/.local/share/karukan-im/segment_learning.tsv`
    pub fn segment_learning_file() -> Option<PathBuf> {
        Self::data_dir().map(|dir| dir.join("segment_learning.tsv"))
    }

    /// Load settings from the default configuration file.
    /// Falls back to embedded default.toml if the config file does not exist.
    pub fn load() -> Result<Self> {
        let Some(config_file) = Self::config_file() else {
            warn!("Could not determine config directory, using defaults");
            return Ok(Self::default());
        };

        if !config_file.exists() {
            debug!("Config file not found, using defaults");
            return Ok(Self::default());
        }

        debug!("Loading config from {:?}", config_file);
        let content = fs::read_to_string(&config_file)?;
        parse_with_defaults(&content)
    }

    /// Load settings from a specific file, merged on top of defaults.
    pub fn load_from(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        parse_with_defaults(&content)
    }

    /// Save settings to the default configuration file
    pub fn save(&self) -> Result<()> {
        let Some(config_file) = Self::config_file() else {
            anyhow::bail!("Could not determine config directory");
        };

        // Create config directory if it doesn't exist
        if let Some(parent) = config_file.parent() {
            fs::create_dir_all(parent)?;
        }

        debug!("Saving config to {:?}", config_file);
        let content = toml::to_string_pretty(self)?;
        fs::write(&config_file, content)?;
        Ok(())
    }

    /// Save settings to a specific file
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert_eq!(settings.conversion.live_num_candidates, 3);
        assert_eq!(settings.conversion.num_candidates, 9);
        assert!(settings.conversion.use_context);
        assert_eq!(settings.conversion.max_context_length, 10);
    }

    #[test]
    fn test_serialize_deserialize() {
        let settings = Settings::default();
        let toml_str = toml::to_string(&settings).unwrap();
        let loaded: Settings = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            loaded.conversion.num_candidates,
            settings.conversion.num_candidates
        );
    }

    #[test]
    fn test_load_from_file() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
[conversion]
num_candidates = 5
use_context = false
"#
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let settings = Settings::load_from(&path).unwrap();
        assert_eq!(settings.conversion.num_candidates, 5);
        assert!(!settings.conversion.use_context);
    }

    #[test]
    fn test_user_dict_dir() {
        let dir = Settings::user_dict_dir();
        // Should return Some on systems with a home directory
        if let Some(dir) = dir {
            assert!(dir.ends_with("user_dicts"));
        }
    }

    #[test]
    fn test_partial_config() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
[conversion]
num_candidates = 3
"#
        )
        .unwrap();

        let path = file.path().to_path_buf();
        let settings = Settings::load_from(&path).unwrap();
        assert_eq!(settings.conversion.num_candidates, 3);
        // Should use default for unspecified values
        assert!(settings.conversion.use_context);
        assert_eq!(settings.conversion.max_context_length, 10);
    }

    #[test]
    fn test_deprecated_conversion_settings_are_ignored() {
        let settings = parse_with_defaults(
            r#"
[conversion]
strategy = "light"
light_model = "jinen-v1-xsmall-q5"
short_input_threshold = 5
beam_width = 2
max_latency_ms = 50
model = "jinen-v1-xsmall-q5"
"#,
        )
        .unwrap();

        assert_eq!(
            settings.conversion.model.as_deref(),
            Some("jinen-v1-xsmall-q5")
        );
        assert_eq!(settings.conversion.num_candidates, 9);

        let serialized = toml::to_string(&settings).unwrap();
        for key in DEPRECATED_CONVERSION_KEYS {
            assert!(!serialized.contains(key));
        }
    }
}
