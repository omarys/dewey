pub mod components;
pub mod theme;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

use crate::app::App;
use components::{
    render_chapters_list, render_details_pane, render_downloads_bar, render_footer, render_header,
    render_help_modal, render_series_list,
};
use theme::Theme;

pub fn render(f: &mut Frame, app: &mut App) {
    let theme = Theme::default();

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(12),   // Body
            Constraint::Length(3), // Active Downloads
            Constraint::Length(1), // Footer / Keybindings
        ])
        .split(f.area());

    render_header(f, main_chunks[0], app, &theme);

    // Body: Split horizontally into Left (Series) and Right (Chapters + Details)
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(42), // Series List
            Constraint::Percentage(58), // Chapters + Details
        ])
        .split(main_chunks[1]);

    render_series_list(f, body_chunks[0], app, &theme);

    // Right column: Split vertically into Chapters table and Series Metadata details
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(65), // Chapters List
            Constraint::Percentage(35), // Details Pane
        ])
        .split(body_chunks[1]);

    render_chapters_list(f, right_chunks[0], app, &theme);
    render_details_pane(f, right_chunks[1], app, &theme);

    render_downloads_bar(f, main_chunks[2], app, &theme);
    render_footer(f, main_chunks[3], app, &theme);

    if app.show_help_modal {
        render_help_modal(f, &theme);
    }
}
