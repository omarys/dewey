#![allow(dead_code)]

use ratatui::style::{Color, Modifier, Style};

pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub muted_fg: Color,
    pub border: Color,
    pub border_focus: Color,
    pub highlight_bg: Color,
    pub highlight_fg: Color,
    pub accent: Color,
    pub accent_alt: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: Color::Reset,
            fg: Color::Rgb(220, 224, 232),
            muted_fg: Color::Rgb(140, 145, 160),
            border: Color::Rgb(70, 75, 90),
            border_focus: Color::Rgb(137, 180, 250), // Soft azure highlight
            highlight_bg: Color::Rgb(45, 50, 68),
            highlight_fg: Color::Rgb(255, 255, 255),
            accent: Color::Rgb(137, 180, 250),
            accent_alt: Color::Rgb(166, 227, 161), // Sage green
            success: Color::Rgb(166, 227, 161),
            warning: Color::Rgb(249, 226, 175), // Warm amber
            error: Color::Rgb(243, 139, 168),   // Soft rose red
        }
    }
}

impl Theme {
    pub fn title(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn block_title_focused(&self) -> Style {
        Style::default()
            .fg(self.border_focus)
            .add_modifier(Modifier::BOLD)
    }

    pub fn block_title_normal(&self) -> Style {
        Style::default().fg(self.muted_fg)
    }

    pub fn active_border(&self) -> Style {
        Style::default().fg(self.border_focus)
    }

    pub fn inactive_border(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn selected_item(&self) -> Style {
        Style::default()
            .bg(self.highlight_bg)
            .fg(self.highlight_fg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn normal_item(&self) -> Style {
        Style::default().fg(self.fg)
    }

    pub fn muted_item(&self) -> Style {
        Style::default().fg(self.muted_fg)
    }

    pub fn success_badge(&self) -> Style {
        Style::default().fg(self.success)
    }

    pub fn warning_badge(&self) -> Style {
        Style::default().fg(self.warning)
    }

    pub fn error_badge(&self) -> Style {
        Style::default().fg(self.error)
    }
}
