#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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

    /// Automatically scan the library directory on startup
    pub auto_scan_on_startup: bool,

    /// Seed sample data if database and library are completely empty
    pub seed_sample_data: bool,
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
            auto_scan_on_startup: true,
            seed_sample_data: false,
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
            let cfg = Config::default();
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(toml_str) = toml::to_string_pretty(&cfg) {
                let _ = fs::write(&path, toml_str);
            }
            Ok(cfg)
        }
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
    }
}
