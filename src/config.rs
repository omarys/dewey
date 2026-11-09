#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StorageProfile {
    #[default]
    Fast,
    Usb,
}

impl StorageProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Usb => "usb",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "usb" | "removable" | "slow" | "low-io" | "sd" => Self::Usb,
            _ => Self::Fast,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Default directory to scan for manga/manhwa chapters and series
    pub library_dir: PathBuf,

    /// Path to SQLite database file
    pub db_path: PathBuf,

    /// Log file destination
    pub log_file: PathBuf,

    /// Path to Continuum executable
    pub continuum_bin: PathBuf,

    /// Path to Labrador scraper executable
    pub labrador_bin: PathBuf,

    /// Settings for searching series through Labrador
    #[serde(default)]
    pub labrador: LabradorConfig,

    /// Editor used to read plain-text (light novel) chapters. Resolution order:
    /// this value, then `$EDITOR`, then `$VISUAL`, then `vi`.
    #[serde(default)]
    pub editor: Option<String>,

    /// Viewer for markdown chapters. Omit to auto-detect Leaf, then Glow, then
    /// fall back to the editor; set to `"editor"` to always use the editor.
    #[serde(default)]
    pub markdown_viewer: Option<String>,

    /// Automatically scan the library directory on startup
    pub auto_scan_on_startup: bool,

    /// Seed sample data if database and library are completely empty
    pub seed_sample_data: bool,

    /// Storage optimization profile ('fast' or 'usb')
    #[serde(default)]
    pub storage_profile: StorageProfile,

    /// Optional override for library scanner worker thread concurrency
    #[serde(default)]
    pub max_scan_concurrency: Option<usize>,

    /// Time window in hours before a series chapter list is considered stale (default: 24)
    #[serde(default = "default_chapter_stale_hours")]
    pub chapter_stale_hours: u64,

    /// Delete a chapter's downloaded archive once it is marked completed, keeping
    /// the chapter record, reading progress, and bookmarks
    #[serde(default)]
    pub delete_after_read: bool,

    /// Settings for importing Komikku/Tachiyomi backups
    #[serde(default)]
    pub komikku: KomikkuConfig,

    /// Where this config was loaded from, so runtime changes can be written back.
    /// Not part of the file format.
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

/// Settings for searching series through Labrador.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabradorConfig {
    /// Providers searched when adding a series, in the order they are shown.
    /// Empty means "every provider Labrador reports".
    ///
    /// ```toml
    /// [labrador]
    /// providers = ["weebcentral", "webtoons"]
    /// ```
    #[serde(default)]
    pub providers: Vec<String>,
}

/// Settings for importing Komikku/Tachiyomi backups.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KomikkuConfig {
    /// Base URL per Komikku source name, used to turn relative chapter links
    /// (e.g. `/chapters/01J76X`) into absolute fetch URLs. Sources missing here
    /// keep their chapters without a fetch URL.
    ///
    /// ```toml
    /// [komikku.sources]
    /// "Weeb Central" = "https://weebcentral.com/"
    /// ```
    #[serde(default)]
    pub sources: std::collections::HashMap<String, String>,
}

fn default_chapter_stale_hours() -> u64 {
    24
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let default_library = Self::detect_default_library(&home);

        Self {
            library_dir: default_library,
            // Canonical user-level locations (XDG-style) so config-less runs
            // never drop databases/logs into arbitrary cwd directories.
            db_path: PathBuf::from(format!("{}/.local/share/dewey/dewey.db", home)),
            log_file: PathBuf::from(format!("{}/.local/state/dewey/dewey.log", home)),
            continuum_bin: PathBuf::from("continuum"),
            labrador_bin: PathBuf::from("labrador"),
            labrador: LabradorConfig::default(),
            editor: None,
            markdown_viewer: None,
            auto_scan_on_startup: true,
            seed_sample_data: false,
            storage_profile: StorageProfile::Fast,
            max_scan_concurrency: None,
            chapter_stale_hours: 24,
            delete_after_read: false,
            komikku: KomikkuConfig::default(),
            source_path: None,
        }
    }
}

impl Config {
    pub fn detect_default_library(home: &str) -> PathBuf {
        let candidates = [
            format!("{}/Documents/Books", home),
            format!("{}/Documents/Manga", home),
            format!("{}/Manga", home),
            format!("{}/Books", home),
        ];

        for cand in &candidates {
            let p = PathBuf::from(cand);
            if p.exists() {
                return p;
            }
        }

        PathBuf::from(format!("{}/Documents/Books", home))
    }

    pub fn auto_detect_storage_profile(path: &Path) -> StorageProfile {
        let p_str = path.to_string_lossy().to_lowercase();
        if p_str.starts_with("/media/")
            || p_str.starts_with("/run/media/")
            || p_str.starts_with("/mnt/")
            || p_str.contains("/usb")
        {
            StorageProfile::Usb
        } else {
            StorageProfile::Fast
        }
    }

    pub fn load_or_create(config_path: Option<&Path>) -> Result<Self> {
        let path = match config_path {
            Some(p) => p.to_path_buf(),
            None => Self::default_config_path(),
        };

        if path.exists() {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config from {:?}", path))?;
            let mut cfg: Config = toml::from_str(&content)
                .with_context(|| format!("Failed to parse config file {:?}", path))?;
            cfg.resolve_tilde();
            cfg.source_path = Some(path.clone());

            // Legacy configs generated before canonical XDG defaults used
            // cwd-relative db_path/log_file ("dewey.db"/"dewey.log"), which
            // duplicated files into every launch directory. Upgrade them to
            // the canonical defaults and persist the fix.
            let defaults = Config::default();
            let mut migrated = false;
            if cfg.db_path.is_relative() {
                cfg.db_path = defaults.db_path;
                migrated = true;
            }
            if cfg.log_file.is_relative() {
                cfg.log_file = defaults.log_file;
                migrated = true;
            }
            if migrated {
                if let Ok(toml_str) = toml::to_string_pretty(&cfg) {
                    let _ = fs::write(&path, toml_str);
                }
            }
            Ok(cfg)
        } else {
            let mut cfg = Config::default();
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            cfg.source_path = Some(path.clone());
            if let Ok(toml_str) = toml::to_string_pretty(&cfg) {
                let _ = fs::write(&path, toml_str);
            }
            Ok(cfg)
        }
    }

    /// Writes this config back to the file it came from (or the default path),
    /// preserving everything already in it. Returns the path written.
    pub fn save(&self) -> Result<PathBuf> {
        let path = self
            .source_path
            .clone()
            .unwrap_or_else(Self::default_config_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory {:?}", parent))?;
        }
        let toml_str = toml::to_string_pretty(self)
            .with_context(|| format!("Failed to serialize config for {:?}", path))?;
        fs::write(&path, toml_str)
            .with_context(|| format!("Failed to write config to {:?}", path))?;
        Ok(path)
    }

    pub fn default_config_path() -> PathBuf {
        // Always the user-level config; never a cwd-relative dewey.toml, so
        // launching from different directories cannot create per-dir configs.
        // Pass -c to use a specific config file explicitly.
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(format!("{}/.config/dewey/config.toml", home))
        } else {
            PathBuf::from("dewey.toml")
        }
    }

    fn resolve_tilde(&mut self) {
        if let Ok(home) = std::env::var("HOME") {
            self.library_dir = expand_tilde(&self.library_dir, &home);
            self.db_path = expand_tilde(&self.db_path, &home);
            self.log_file = expand_tilde(&self.log_file, &home);
        }
    }
}

fn expand_tilde(path: &Path, home: &str) -> PathBuf {
    let path_str = path.to_string_lossy();
    if let Some(stripped) = path_str.strip_prefix("~/") {
        PathBuf::from(format!("{}/{}", home, stripped))
    } else if path_str == "~" {
        PathBuf::from(home)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_labrador_providers_round_trip() {
        let mut cfg = Config::default();
        cfg.labrador.providers = vec!["weebcentral".to_string(), "webtoons".to_string()];

        let toml_str = toml::to_string_pretty(&cfg).expect("serialize");
        assert!(toml_str.contains("[labrador]"), "got: {toml_str}");
        assert!(
            !toml_str.contains("source_path"),
            "runtime-only path must not be written: {toml_str}"
        );

        let back: Config = toml::from_str(&toml_str).expect("parse");
        assert_eq!(back.labrador.providers, cfg.labrador.providers);
        assert!(back.source_path.is_none());
    }

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.continuum_bin, PathBuf::from("continuum"));
        assert_eq!(cfg.labrador_bin, PathBuf::from("labrador"));
        // Canonical defaults are user-level, never cwd-relative.
        assert!(cfg
            .db_path
            .to_string_lossy()
            .contains(".local/share/dewey/dewey.db"));
        assert!(cfg
            .log_file
            .to_string_lossy()
            .contains(".local/state/dewey/dewey.log"));
        assert!(cfg.db_path.is_absolute());
        assert!(cfg.log_file.is_absolute());
        assert_eq!(cfg.storage_profile, StorageProfile::Fast);
        assert!(!cfg.delete_after_read);
    }

    #[test]
    fn test_storage_profile_parsing_and_detection() {
        assert_eq!(StorageProfile::from_str_loose("usb"), StorageProfile::Usb);
        assert_eq!(StorageProfile::from_str_loose("slow"), StorageProfile::Usb);
        assert_eq!(StorageProfile::from_str_loose("fast"), StorageProfile::Fast);

        assert_eq!(
            Config::auto_detect_storage_profile(Path::new("/media/user/DRIVE/Manga")),
            StorageProfile::Usb
        );
        assert_eq!(
            Config::auto_detect_storage_profile(Path::new("/run/media/user/DRIVE")),
            StorageProfile::Usb
        );
        assert_eq!(
            Config::auto_detect_storage_profile(Path::new("/mnt/usb_disk")),
            StorageProfile::Usb
        );
        assert_eq!(
            Config::auto_detect_storage_profile(Path::new("/home/user/Documents/Manga")),
            StorageProfile::Fast
        );
    }

    #[test]
    fn editor_is_optional_and_parses_with_arguments() {
        assert!(
            Config::default().editor.is_none(),
            "no editor means $EDITOR"
        );

        let cfg: Config = toml::from_str(
            r#"
            library_dir = "/tmp/library"
            db_path = "/tmp/dewey.db"
            log_file = "/tmp/dewey.log"
            continuum_bin = "continuum"
            labrador_bin = "labrador"
            auto_scan_on_startup = true
            seed_sample_data = false
            editor = "nvim -u NONE"
        "#,
        )
        .expect("editor parses");
        assert_eq!(cfg.editor.as_deref(), Some("nvim -u NONE"));
        assert!(
            cfg.markdown_viewer.is_none(),
            "viewer auto-detects by default"
        );
    }

    #[test]
    fn komikku_sources_parse_from_config_and_default_to_empty() {
        assert!(Config::default().komikku.sources.is_empty());

        let minimal = r#"
            library_dir = "/tmp/library"
            db_path = "/tmp/dewey.db"
            log_file = "/tmp/dewey.log"
            continuum_bin = "continuum"
            labrador_bin = "labrador"
            auto_scan_on_startup = true
            seed_sample_data = false
        "#;
        let cfg: Config = toml::from_str(minimal).expect("config without komikku parses");
        assert!(cfg.komikku.sources.is_empty());

        let with_sources = format!(
            "{minimal}\n[komikku.sources]\n\"Weeb Central\" = \"https://weebcentral.com/\"\n\"Manga District\" = \"https://mangadistrict.com/\"\n"
        );
        let cfg: Config = toml::from_str(&with_sources).expect("komikku sources parse");
        assert_eq!(
            cfg.komikku.sources.get("Weeb Central").map(String::as_str),
            Some("https://weebcentral.com/")
        );
        assert_eq!(cfg.komikku.sources.len(), 2);
    }
}
