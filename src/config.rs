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
            db_path: PathBuf::from("dewey.db"),
            log_file: PathBuf::from("dewey.log"),
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
        if Path::new("dewey.toml").exists() {
            PathBuf::from("dewey.toml")
        } else if let Ok(home) = std::env::var("HOME") {
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
    }
}
