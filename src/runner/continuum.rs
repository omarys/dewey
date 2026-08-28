use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tracing::{error, info, warn};

/// One chapter's progress as reported by the reader, page numbers LOCAL to
/// that chapter's own archive (never a global page offset).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterProgressPayload {
    pub file: PathBuf,
    pub last_page: i64,
    pub completed: bool,
}

/// The JSON payload emitted by Continuum on stdout upon closing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContinuumExitPayload {
    pub last_page: i64,
    pub completed: bool,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    /// Every chapter the user actually read this session, in reading order;
    /// absent/None keeps the legacy single-chapter contract working.
    #[serde(default)]
    pub chapters: Option<Vec<ChapterProgressPayload>>,
}

impl ContinuumExitPayload {
    /// Generates a celebratory or progress summary message upon session close
    pub fn completion_message(&self, chapter_number: f64) -> String {
        if let Some(msg) = &self.message {
            msg.clone()
        } else if self.completed {
            format!(
                "🎉 Chapter {:.1} completed! Marked as read [✓]",
                chapter_number
            )
        } else {
            format!(
                "📖 Saved progress: Chapter {:.1} @ page {}",
                chapter_number, self.last_page
            )
        }
    }
}

pub struct ContinuumRunner {
    binary_path: PathBuf,
    storage_profile: Option<String>,
}

impl Default for ContinuumRunner {
    fn default() -> Self {
        Self::new("continuum")
    }
}

impl ContinuumRunner {
    pub fn new(binary_path: impl Into<PathBuf>) -> Self {
        Self {
            binary_path: binary_path.into(),
            storage_profile: None,
        }
    }

    pub fn with_storage_profile(mut self, profile: &str) -> Self {
        self.storage_profile = Some(profile.to_string());
        self
    }

    /// Builds the Command configured with clean --file, --page, --mode, and --storage-profile flags.
    pub fn build_command(&self, file_path: &Path, last_page: i64, mode: Option<&str>) -> Command {
        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("--file")
            .arg(file_path)
            .arg("--page")
            .arg(last_page.to_string())
            .arg("--mode")
            .arg(mode.unwrap_or("webtoon"));
        if let Some(prof) = &self.storage_profile {
            cmd.arg("--storage-profile").arg(prof);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    /// Spawns Continuum synchronously.
    /// Note: The caller MUST suspend/restore the TUI raw mode / alternate screen
    /// around this call so that stdout/signals are clean.
    pub fn spawn_and_wait(
        &self,
        file_path: &Path,
        last_page: i64,
        mode: Option<&str>,
    ) -> Result<ContinuumExitPayload> {
        if !file_path.exists() {
            return Err(anyhow!("File does not exist: {:?}", file_path));
        }

        info!(
            binary = ?self.binary_path,
            file = ?file_path,
            page = last_page,
            mode = ?mode,
            "Launching Continuum reader process"
        );

        let mut cmd = self.build_command(file_path, last_page, mode);

        let output = cmd.output().with_context(|| {
            format!(
                "Failed to execute Continuum process: {:?}. Ensure 'continuum' is in PATH.",
                self.binary_path
            )
        })?;

        let stdout_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr_str = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            warn!(
                status = ?output.status.code(),
                stderr = %stderr_str,
                "Continuum exited with non-zero exit code"
            );
        }

        info!(
            stdout = %stdout_str,
            "Continuum process terminated. Parsing exit payload."
        );

        Self::parse_payload(&stdout_str, last_page)
    }

    /// Parses the JSON payload from Continuum's stdout.
    /// Handles cases where stdout contains surrounding log lines or pure JSON.
    pub fn parse_payload(raw_stdout: &str, fallback_page: i64) -> Result<ContinuumExitPayload> {
        if raw_stdout.is_empty() {
            warn!("Continuum produced empty stdout. Using fallback progress.");
            return Ok(ContinuumExitPayload {
                last_page: fallback_page,
                completed: false,
                mode: None,
                message: None,
                chapters: None,
            });
        }

        // Attempt 1: Direct JSON deserialization
        if let Ok(payload) = serde_json::from_str::<ContinuumExitPayload>(raw_stdout) {
            return Ok(payload);
        }

        // Attempt 2: Search for the JSON object substring in stdout lines
        for line in raw_stdout.lines().rev() {
            let trimmed = line.trim();
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                if let Ok(payload) = serde_json::from_str::<ContinuumExitPayload>(trimmed) {
                    return Ok(payload);
                }
            }
        }

        error!(
            raw_stdout = %raw_stdout,
            "Could not parse valid ContinuumExitPayload from stdout."
        );
        Err(anyhow!(
            "Failed to parse Continuum JSON payload from stdout: '{}'",
            raw_stdout
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_command_clean_args() {
        let runner = ContinuumRunner::new("continuum");
        let test_path = Path::new("/tmp/test.cbz");
        let cmd = runner.build_command(test_path, 42, Some("manga"));

        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        assert_eq!(
            args,
            vec!["--file", "/tmp/test.cbz", "--page", "42", "--mode", "manga"]
        );
    }

    #[test]
    fn test_build_command_with_storage_profile() {
        let runner = ContinuumRunner::new("continuum").with_storage_profile("usb");
        let test_path = Path::new("/tmp/test.cbz");
        let cmd = runner.build_command(test_path, 10, Some("webtoon"));

        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        assert_eq!(
            args,
            vec![
                "--file",
                "/tmp/test.cbz",
                "--page",
                "10",
                "--mode",
                "webtoon",
                "--storage-profile",
                "usb"
            ]
        );
    }

    #[test]
    fn test_parse_payload_with_chapters() {
        let json = r#"{"last_page": 15, "completed": true, "chapters": [{"file": "/manga/a.cbz", "last_page": 15, "completed": true}, {"file": "/manga/b.cbz", "last_page": 4, "completed": false}]}"#;
        let payload = ContinuumRunner::parse_payload(json, 0).unwrap();
        assert_eq!(
            payload.chapters,
            Some(vec![
                ChapterProgressPayload {
                    file: PathBuf::from("/manga/a.cbz"),
                    last_page: 15,
                    completed: true,
                },
                ChapterProgressPayload {
                    file: PathBuf::from("/manga/b.cbz"),
                    last_page: 4,
                    completed: false,
                },
            ])
        );
        // Legacy fields still present.
        assert_eq!(payload.last_page, 15);
        assert!(payload.completed);
    }

    #[test]
    fn test_parse_payload_without_chapters() {
        let json = r#"{"last_page": 3, "completed": false}"#;
        let payload = ContinuumRunner::parse_payload(json, 0).unwrap();
        assert_eq!(payload.chapters, None);
        assert_eq!(payload.last_page, 3);
        assert!(!payload.completed);
    }

    #[test]
    fn test_parse_payload_direct() {
        let json = r#"{"last_page": 45, "completed": false, "mode": "manga"}"#;
        let payload = ContinuumRunner::parse_payload(json, 0).unwrap();
        assert_eq!(
            payload,
            ContinuumExitPayload {
                last_page: 45,
                completed: false,
                mode: Some("manga".to_string()),
                message: None,
                chapters: None,
            }
        );
        assert_eq!(
            payload.completion_message(10.0),
            "📖 Saved progress: Chapter 10.0 @ page 45"
        );
    }

    #[test]
    fn test_parse_payload_completed_with_message() {
        let json = r#"{"last_page": 60, "completed": true, "message": "🎉 Great read! Chapter 10 completed."}"#;
        let payload = ContinuumRunner::parse_payload(json, 0).unwrap();
        assert_eq!(
            payload.completion_message(10.0),
            "🎉 Great read! Chapter 10 completed."
        );
    }

    #[test]
    fn test_parse_payload_with_logs() {
        let output = "QML debugging enabled\nLoaded 48 pages\n{\"last_page\": 12, \"completed\": true, \"mode\": \"webtoon\"}\nExiting.";
        let payload = ContinuumRunner::parse_payload(output, 0).unwrap();
        assert_eq!(
            payload,
            ContinuumExitPayload {
                last_page: 12,
                completed: true,
                mode: Some("webtoon".to_string()),
                message: None,
                chapters: None,
            }
        );
        assert_eq!(
            payload.completion_message(5.0),
            "🎉 Chapter 5.0 completed! Marked as read [✓]"
        );
    }
}
