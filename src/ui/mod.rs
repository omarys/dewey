pub mod components;
pub mod theme;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

use crate::app::App;
use components::{
    render_action_bar, render_chapters_list, render_details_pane, render_downloads_bar,
    render_footer, render_header, render_help_modal, render_series_list,
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
            Constraint::Length(1), // Touch action bar
            Constraint::Length(1), // Footer / Keybindings
        ])
        .split(f.area());

    render_header(f, main_chunks[0], app, &theme);

    // Body: landscape = side-by-side panes; portrait = stacked full-width panes
    // so narrow touchscreen columns stay usable.
    let body = main_chunks[1];
    let is_portrait = body.height > body.width;
    let (series_rect, chapters_rect, details_rect) = if is_portrait {
        let body_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(45), // Series List
                Constraint::Percentage(45), // Chapters List
                Constraint::Percentage(10), // Details Pane
            ])
            .split(body);
        (body_chunks[0], body_chunks[1], body_chunks[2])
    } else {
        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(42), // Series List
                Constraint::Percentage(58), // Chapters + Details
            ])
            .split(body);

        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(65), // Chapters List
                Constraint::Percentage(35), // Details Pane
            ])
            .split(body_chunks[1]);
        (body_chunks[0], right_chunks[0], right_chunks[1])
    };

    // Publish hit-test areas for touch/tap input handling.
    app.series_rect = Some(series_rect);
    app.chapters_rect = Some(chapters_rect);

    render_series_list(f, series_rect, app, &theme);
    render_chapters_list(f, chapters_rect, app, &theme);
    render_details_pane(f, details_rect, app, &theme);

    render_downloads_bar(f, main_chunks[2], app, &theme);
    render_action_bar(f, main_chunks[3], app, &theme);
    render_footer(f, main_chunks[4], app, &theme);

    if app.show_help_modal {
        render_help_modal(f, &theme);
    }
}
