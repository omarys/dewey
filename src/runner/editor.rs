//! Reading plain-text (light novel) chapters.
//!
//! A novel chapter is a text file, so there is nothing to render: Dewey suspends
//! the TUI, hands the file to an external viewer, and waits. Markdown gets a
//! markdown viewer when one is available (Leaf, then Glow); everything else — and
//! markdown when neither viewer works — goes to the user's editor.
//!
//! Progress is recorded by [`crate::db::Database::mark_chapter_opened`]
//! afterwards: the viewer owns the reading position, so Dewey only learns "this
//! chapter was opened".

use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use tracing::warn;

/// Used when neither the config nor the environment names an editor.
pub const FALLBACK_EDITOR: &str = "vi";

/// Markdown viewers, in preference order. `glow -p` pages its output, which is
/// what makes it usable from a suspended TUI (plain `glow` prints and exits,
/// scrolling the chapter away before the TUI redraws).
const MARKDOWN_CHAIN: [&str; 2] = ["leaf", "glow -p"];

/// Configured `markdown_viewer` value that means "no viewer, use the editor".
const VIEWER_OFF: &str = "editor";

pub struct EditorRunner {
    /// Fallback editor for non-markdown chapters and when no viewer works.
    command: String,
    /// Resolved once at startup: `None` means markdown also goes to the editor.
    markdown_viewer: Option<String>,
}

impl EditorRunner {
    /// `configured_editor` is the `editor` config key; `configured_viewer` is
    /// `markdown_viewer`. Editor resolution: config, `$EDITOR`, `$VISUAL`,
    /// [`FALLBACK_EDITOR`]. Viewer resolution: config, else [`MARKDOWN_CHAIN`]
    /// probed for a working binary.
    pub fn new(configured_editor: Option<&str>, configured_viewer: Option<&str>) -> Self {
        Self {
            command: Self::pick(
                configured_editor,
                std::env::var("EDITOR").ok(),
                std::env::var("VISUAL").ok(),
            ),
            markdown_viewer: Self::pick_markdown_viewer(configured_viewer, Self::command_works),
        }
    }

    /// Editor resolution order, as a pure function so it can be tested without
    /// touching the process environment.
    fn pick(configured: Option<&str>, editor: Option<String>, visual: Option<String>) -> String {
        let non_empty = |value: Option<String>| value.filter(|v| !v.trim().is_empty());
        configured
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .or_else(|| non_empty(editor))
            .or_else(|| non_empty(visual))
            .unwrap_or_else(|| FALLBACK_EDITOR.to_string())
    }

    /// Viewer resolution order. A configured value is taken as-is (the user
    /// insists), except `"editor"`, which opts out of the chain.
    fn pick_markdown_viewer(
        configured: Option<&str>,
        usable: impl Fn(&str) -> bool,
    ) -> Option<String> {
        match configured.map(str::trim).filter(|v| !v.is_empty()) {
            Some(command) if command.eq_ignore_ascii_case(VIEWER_OFF) => None,
            Some(command) => Some(command.to_string()),
            None => MARKDOWN_CHAIN
                .iter()
                .find(|command| usable(command))
                .map(|command| command.to_string()),
        }
    }

    /// Whether `command` really runs here.
    ///
    /// Presence on `PATH` is not enough: a version-manager shim (mise, asdf) is
    /// on `PATH` and still exits non-zero with "No version is set for shim",
    /// which would replace the chapter with an error message. So each candidate
    /// is actually executed once, at startup.
    fn command_works(command: &str) -> bool {
        let program = command.split_whitespace().next().unwrap_or(command);
        Command::new(program)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    /// True for markdown files, which may open in a markdown viewer.
    pub fn is_markdown(path: &Path) -> bool {
        matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .as_deref(),
            Some("md") | Some("markdown")
        )
    }

    /// The command that will actually open `file`, for toasts and logs.
    pub fn command_for(&self, file: &Path) -> &str {
        match (&self.markdown_viewer, Self::is_markdown(file)) {
            (Some(viewer), true) => viewer,
            _ => &self.command,
        }
    }

    /// Splits a command so values like `glow -p` or `nvim -u NONE` work.
    fn parts(command: &str) -> Vec<String> {
        command.split_whitespace().map(str::to_string).collect()
    }

    /// Opens `file` in the resolved viewer (or editor) with inherited stdio and
    /// waits for it to exit.
    ///
    /// A non-zero exit (e.g. vim's `:cq`) is only warned about: the chapter was
    /// still opened, so the caller should keep recording that.
    pub fn open_and_wait(&self, file: &Path) -> Result<()> {
        if !file.exists() {
            return Err(anyhow!("Chapter file does not exist: {:?}", file));
        }

        let command = self.command_for(file);
        let mut parts = Self::parts(command);
        if parts.is_empty() {
            return Err(anyhow!("No editor configured for reading text chapters"));
        }
        let program = parts.remove(0);

        let status = Command::new(&program)
            .args(&parts)
            .arg(file)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| anyhow!("Failed to launch `{}`: {}", program, e))?;

        if !status.success() {
            warn!(
                command = %command,
                file = ?file,
                status = %status,
                "Chapter viewer exited with a non-zero status"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_runner(editor: &str, viewer: Option<&str>) -> EditorRunner {
        EditorRunner {
            command: editor.to_string(),
            markdown_viewer: viewer.map(str::to_string),
        }
    }

    #[test]
    fn configured_editor_wins_over_environment() {
        let picked = EditorRunner::pick(
            Some("nvim -u NONE"),
            Some("nano".to_string()),
            Some("emacs".to_string()),
        );
        assert_eq!(picked, "nvim -u NONE");
    }

    #[test]
    fn editor_env_beats_visual() {
        let picked = EditorRunner::pick(None, Some("nano".to_string()), Some("emacs".to_string()));
        assert_eq!(picked, "nano");
    }

    #[test]
    fn visual_is_used_when_editor_is_unset() {
        let picked = EditorRunner::pick(None, None, Some("emacs".to_string()));
        assert_eq!(picked, "emacs");
    }

    #[test]
    fn blank_values_fall_through_to_vi() {
        let picked = EditorRunner::pick(Some("   "), Some(String::new()), Some(" ".to_string()));
        assert_eq!(picked, FALLBACK_EDITOR);
    }

    #[test]
    fn markdown_is_detected_by_extension() {
        assert!(EditorRunner::is_markdown(Path::new("Chapter 001.md")));
        assert!(EditorRunner::is_markdown(Path::new("Chapter 002.MD")));
        assert!(EditorRunner::is_markdown(Path::new("notes.markdown")));
        assert!(!EditorRunner::is_markdown(Path::new("Chapter 003.txt")));
        assert!(!EditorRunner::is_markdown(Path::new("Chapter 004.cbz")));
    }

    #[test]
    fn leaf_is_preferred_over_glow() {
        let picked = EditorRunner::pick_markdown_viewer(None, |command| {
            command.starts_with("leaf") || command.starts_with("glow")
        });
        assert_eq!(picked.as_deref(), Some("leaf"));
    }

    #[test]
    fn glow_is_used_when_leaf_is_missing() {
        let picked =
            EditorRunner::pick_markdown_viewer(None, |command| command.starts_with("glow"));
        assert_eq!(picked.as_deref(), Some("glow -p"));
    }

    #[test]
    fn no_working_viewer_falls_back_to_the_editor() {
        let picked = EditorRunner::pick_markdown_viewer(None, |_| false);
        assert_eq!(picked, None);
    }

    #[test]
    fn configured_viewer_is_taken_as_is_without_probing() {
        let picked = EditorRunner::pick_markdown_viewer(Some("mdcat --paginate"), |_| false);
        assert_eq!(picked.as_deref(), Some("mdcat --paginate"));
    }

    #[test]
    fn configured_editor_value_opts_out_of_the_chain() {
        let picked = EditorRunner::pick_markdown_viewer(Some("editor"), |_| true);
        assert_eq!(picked, None);
    }

    #[test]
    fn markdown_uses_the_viewer_and_plain_text_the_editor() {
        let runner = make_runner("nvim", Some("leaf"));
        assert_eq!(runner.command_for(Path::new("Ch 1.md")), "leaf");
        assert_eq!(runner.command_for(Path::new("Ch 1.txt")), "nvim");

        // Without a viewer, markdown also goes to the editor.
        let runner = make_runner("nvim", None);
        assert_eq!(runner.command_for(Path::new("Ch 1.md")), "nvim");
    }

    #[test]
    fn command_with_arguments_is_split() {
        assert_eq!(
            EditorRunner::parts("code -w"),
            vec!["code".to_string(), "-w".to_string()]
        );
        assert_eq!(
            EditorRunner::parts("glow -p"),
            vec!["glow".to_string(), "-p".to_string()]
        );
    }

    /// A configured viewer is never probed, but auto-detection must reject a
    /// binary that is on PATH and cannot run (a version-manager shim).
    #[test]
    fn probing_rejects_a_binary_that_cannot_run() {
        assert!(EditorRunner::command_works("sh"));
        assert!(!EditorRunner::command_works("dewey-no-such-viewer-xyz"));
    }

    #[test]
    fn missing_file_is_an_error_not_a_spawn() {
        let runner = EditorRunner::new(Some("true"), None);
        let err = runner
            .open_and_wait(Path::new("/nonexistent/dewey-chapter.txt"))
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }
}
