use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{error, info, warn};

use crate::event::{AppEvent, DownloadSuccessPayload};

static TASK_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabradorResultPayload {
    pub file_path: PathBuf,
    pub page_count: Option<i64>,
    pub fetch_url: Option<String>,
    pub series_fetch_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedSeriesInfo {
    pub title: String,
    pub url: String,
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabradorChapterItem {
    pub id: String,
    pub series_id: Option<String>,
    pub title: String,
    pub url: String,
    pub number: Option<f64>,
    pub index: usize,
    pub original_label: Option<String>,
}

#[derive(Clone)]
pub struct LabradorRunner {
    binary_path: PathBuf,
}

impl Default for LabradorRunner {
    fn default() -> Self {
        Self::new("labrador")
    }
}

impl LabradorRunner {
    pub fn new(binary_path: impl Into<PathBuf>) -> Self {
        Self {
            binary_path: binary_path.into(),
        }
    }

    pub fn resolve_binary(&self) -> PathBuf {
        if self.binary_path != Path::new("labrador") && self.binary_path.exists() {
            return self.binary_path.clone();
        }

        if let Ok(home) = std::env::var("HOME") {
            let candidates = [
                format!("{}/.local/bin/labrador", home),
                format!("{}/.cargo/bin/labrador", home),
                format!("{}/go/bin/labrador", home),
                format!("{}/.local/share/mise/shims/labrador", home),
            ];
            for cand in candidates {
                let p = PathBuf::from(&cand);
                if p.exists() {
                    return p;
                }
            }
        }

        self.binary_path.clone()
    }

    /// Launches Labrador's interactive TUI to search all feeds, allowing the user to select
    /// a series. If query is Some, searches for that title; if None, opens directly in search input.
    pub fn spawn_interactive_select_series(
        &self,
        query: Option<&str>,
    ) -> Result<Option<SelectedSeriesInfo>> {
        let temp_path =
            std::env::temp_dir().join(format!("dewey_labrador_sel_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&temp_path);

        let bin = self.resolve_binary();
        let mut cmd = std::process::Command::new(&bin);
        cmd.arg("select")
            .arg("--json")
            .arg("--output-file")
            .arg(&temp_path);

        if let Some(q) = query {
            cmd.arg("--query").arg(q);
        }

        cmd.stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());

        let status = cmd.status().with_context(|| {
            format!(
                "Failed to run {:?}. Ensure labrador is installed.",
                self.binary_path
            )
        })?;

        if !status.success() {
            let _ = std::fs::remove_file(&temp_path);
            return Ok(None);
        }

        if let Ok(content) = std::fs::read_to_string(&temp_path) {
            let _ = std::fs::remove_file(&temp_path);
            let trimmed = content.trim();
            if let Ok(info) = serde_json::from_str::<SelectedSeriesInfo>(trimmed) {
                if !info.url.is_empty() {
                    return Ok(Some(info));
                }
            }
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                return Ok(Some(SelectedSeriesInfo {
                    title: query.unwrap_or("").to_string(),
                    url: trimmed.to_string(),
                    provider: None,
                }));
            }
        }

        let _ = std::fs::remove_file(&temp_path);
        Ok(None)
    }

    /// Launches Labrador's interactive TUI to search all feeds for series_title,
    /// allowing the user to select the matching series and returning its URL.
    pub fn spawn_interactive_select(&self, series_title: &str) -> Result<Option<String>> {
        match self.spawn_interactive_select_series(Some(series_title))? {
            Some(info) => Ok(Some(info.url)),
            None => Ok(None),
        }
    }

    /// Fetches the complete remote chapter listing for a series URL via `labrador chapters --url <url> --json`.
    pub fn fetch_series_chapters(&self, series_url: &str) -> Result<Vec<LabradorChapterItem>> {
        let bin = self.resolve_binary();
        let mut cmd = std::process::Command::new(&bin);
        cmd.arg("chapters")
            .arg("--url")
            .arg(series_url)
            .arg("--json");

        let output = cmd.output().with_context(|| {
            format!(
                "Failed to execute {:?} chapters. Ensure labrador is installed.",
                self.binary_path
            )
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Labrador chapters error: {}", stderr));
        }

        let chapters: Vec<LabradorChapterItem> = serde_json::from_slice(&output.stdout)
            .context("Failed to parse JSON output from labrador chapters")?;

        Ok(chapters)
    }

    /// Launches Labrador's interactive TUI directly into the chapter listing for a series,
    /// allowing the user to view and queue multiple chapter downloads.
    pub fn spawn_interactive_chapters(
        &self,
        series_url: &str,
        series_title: Option<&str>,
        output_dir: Option<&Path>,
    ) -> Result<()> {
        let bin = self.resolve_binary();
        let mut cmd = std::process::Command::new(&bin);
        cmd.arg("open").arg("--url").arg(series_url);

        if let Some(title) = series_title {
            cmd.arg("--title").arg(title);
        }
        if let Some(out) = output_dir {
            cmd.arg("--series-dir").arg(out);
        }

        cmd.stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());

        let status = cmd.status().with_context(|| {
            format!(
                "Failed to run {:?} open. Ensure labrador is installed.",
                self.binary_path
            )
        })?;

        if !status.success() {
            warn!(?status, "Labrador open exited with non-zero status");
        }

        Ok(())
    }

    /// Spawns an asynchronous download task using tokio::spawn.
    /// If fetch_url is present, it is passed to Labrador.
    /// If fetch_url is missing, Labrador discovers/resolves it and returns it.
    pub fn spawn_fetch(
        &self,
        event_tx: UnboundedSender<AppEvent>,
        series_id: i64,
        series_title: String,
        chapter_number: f64,
        fetch_url: Option<String>,
        series_dir: Option<PathBuf>,
    ) -> u64 {
        let task_id = TASK_COUNTER.fetch_add(1, Ordering::SeqCst);
        let runner = self.clone();

        let _ = event_tx.send(AppEvent::DownloadStarted {
            task_id,
            series_id,
            series_title: series_title.clone(),
            chapter_number,
        });

        tokio::spawn(async move {
            info!(
                task_id,
                series = %series_title,
                chapter = chapter_number,
                url = ?fetch_url,
                "Starting background Labrador fetch task"
            );

            match runner
                .execute_fetch(
                    &series_title,
                    chapter_number,
                    fetch_url.as_deref(),
                    series_dir.as_deref(),
                )
                .await
            {
                Ok(result) => {
                    info!(
                        task_id,
                        series = %series_title,
                        chapter = chapter_number,
                        file = ?result.file_path,
                        ret_url = ?result.fetch_url,
                        "Labrador fetch completed successfully"
                    );

                    let _ = event_tx.send(AppEvent::DownloadSuccess(DownloadSuccessPayload {
                        task_id,
                        series_id,
                        chapter_number,
                        file_path: result.file_path,
                        page_count: result.page_count,
                        fetch_url: result.fetch_url,
                        series_fetch_url: result.series_fetch_url,
                    }));
                }
                Err(err) => {
                    error!(
                        task_id,
                        series = %series_title,
                        chapter = chapter_number,
                        error = %err,
                        "Labrador fetch failed"
                    );

                    let _ = event_tx.send(AppEvent::DownloadFailed {
                        task_id,
                        series_id,
                        series_title,
                        chapter_number,
                        error: err.to_string(),
                    });
                }
            }
        });

        task_id
    }

    /// Internal async execution of the labrador CLI command
    async fn execute_fetch(
        &self,
        series_title: &str,
        chapter_number: f64,
        fetch_url: Option<&str>,
        series_dir: Option<&Path>,
    ) -> Result<LabradorResultPayload> {
        let bin = self.resolve_binary();
        let mut cmd = Command::new(&bin);
        cmd.arg("fetch")
            .arg("--series")
            .arg(series_title)
            .arg("--chapter")
            .arg(chapter_number.to_string());

        if let Some(url) = fetch_url {
            cmd.arg("--url").arg(url);
        }

        if let Some(dir) = series_dir {
            cmd.arg("--series-dir").arg(dir);
        }

        let output = match tokio::time::timeout(std::time::Duration::from_secs(120), cmd.output()).await {
            Ok(res) => res.with_context(|| {
                format!(
                    "Failed to execute Labrador binary at {:?}. Ensure 'labrador' is installed/in PATH.",
                    self.binary_path
                )
            })?,
            Err(_) => {
                return Err(anyhow!(
                    "Labrador fetch timed out after 120s for '{}' Ch. {:.1}",
                    series_title,
                    chapter_number
                ));
            }
        };

        let stdout_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr_str = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            return Err(anyhow!(
                "Labrador exited with code {:?}: {}",
                output.status.code(),
                if !stderr_str.is_empty() {
                    stderr_str
                } else {
                    stdout_str
                }
            ));
        }

        Self::parse_output(&stdout_str, series_title, chapter_number)
    }

    /// Parses the stdout from Labrador, extracting file path, page count, and returned fetch URLs
    pub fn parse_output(
        raw_stdout: &str,
        series_title: &str,
        chapter_number: f64,
    ) -> Result<LabradorResultPayload> {
        // Attempt 1: Direct JSON payload
        if let Ok(payload) = serde_json::from_str::<LabradorResultPayload>(raw_stdout) {
            return Ok(payload);
        }

        // Attempt 2: Search for JSON object in stdout lines
        for line in raw_stdout.lines().rev() {
            let trimmed = line.trim();
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                if let Ok(payload) = serde_json::from_str::<LabradorResultPayload>(trimmed) {
                    return Ok(payload);
                }
            }
        }

        // Attempt 3: Look for a file path line and optional URL line
        let mut detected_path = None;
        let mut detected_url = None;

        for line in raw_stdout.lines() {
            let trimmed = line.trim();
            if trimmed.ends_with(".cbz")
                || trimmed.ends_with(".zip")
                || trimmed.ends_with(".epub")
                || (trimmed.starts_with('/') && Path::new(trimmed).exists())
            {
                let path_str = if let Some(idx) = trimmed.find('/') {
                    &trimmed[idx..]
                } else {
                    trimmed
                };
                detected_path = Some(PathBuf::from(path_str));
            } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                detected_url = Some(trimmed.to_string());
            } else if let Some(url_str) = trimmed.strip_prefix("URL:") {
                detected_url = Some(url_str.trim().to_string());
            } else if let Some(url_str) = trimmed.strip_prefix("Fetch URL:") {
                detected_url = Some(url_str.trim().to_string());
            }
        }

        if let Some(file_path) = detected_path {
            return Ok(LabradorResultPayload {
                file_path,
                page_count: None,
                fetch_url: detected_url,
                series_fetch_url: None,
            });
        }

        // Fallback default convention if stdout was purely informational
        let sanitized_series = series_title.replace(' ', "_").to_lowercase();
        let fallback_path = PathBuf::from(format!(
            "/tmp/manga/{}_c{:.1}.cbz",
            sanitized_series, chapter_number
        ));

        warn!(
            raw_stdout = %raw_stdout,
            fallback = ?fallback_path,
            "Could not parse explicit path from Labrador stdout, using fallback target path"
        );

        Ok(LabradorResultPayload {
            file_path: fallback_path,
            page_count: None,
            fetch_url: detected_url,
            series_fetch_url: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_labrador_json_output_with_url() {
        let json = r#"{"file_path": "/tmp/manga/Solo_Leveling_105.cbz", "page_count": 48, "fetch_url": "https://manga.example/solo/105", "series_fetch_url": "https://manga.example/solo"}"#;
        let result = LabradorRunner::parse_output(json, "Solo Leveling", 105.0).unwrap();
        assert_eq!(
            result.file_path,
            PathBuf::from("/tmp/manga/Solo_Leveling_105.cbz")
        );
        assert_eq!(result.page_count, Some(48));
        assert_eq!(
            result.fetch_url,
            Some("https://manga.example/solo/105".to_string())
        );
        assert_eq!(
            result.series_fetch_url,
            Some("https://manga.example/solo".to_string())
        );
    }

    #[test]
    fn test_parse_labrador_plain_text_path_and_url() {
        let output = "Downloading Solo Leveling...\nFetch URL: https://manga.example/solo/105\nSaved chapter to: /storage/manga/Solo_Leveling_105.cbz\nDone in 2.3s";
        let result = LabradorRunner::parse_output(output, "Solo Leveling", 105.0).unwrap();
        assert_eq!(
            result.file_path,
            PathBuf::from("/storage/manga/Solo_Leveling_105.cbz")
        );
        assert_eq!(
            result.fetch_url,
            Some("https://manga.example/solo/105".to_string())
        );
    }
}
