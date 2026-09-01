pub mod components;
pub mod theme;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

use crate::app::{ActivePane, App};
use components::{
    render_action_bar, render_category_modal, render_chapters_list, render_details_pane,
    render_downloads_bar, render_header, render_help_modal, render_portrait_tab_bar,
    render_series_list,
};
use theme::Theme;

pub fn render(f: &mut Frame, app: &mut App) {
    let theme = Theme::default();
    let area = f.area();
    let is_portrait = area.height > area.width;

    if is_portrait {
        let has_downloads = !app.download_jobs.is_empty();
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),                                 // Header
                Constraint::Length(3),                                 // Portrait Tab Switcher
                Constraint::Min(8),                                    // Active List + Details
                Constraint::Length(if has_downloads { 3 } else { 0 }), // Downloads
                Constraint::Length(2), // Unified 2-row Touch Action Pad
            ])
            .split(area);

        render_header(f, main_chunks[0], app, &theme);
        render_portrait_tab_bar(f, main_chunks[1], app, &theme);

        let body = main_chunks[2];
        let body_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(70), // Active List (Series or Chapters)
                Constraint::Percentage(30), // Details Pane
            ])
            .split(body);

        let list_rect = body_chunks[0];
        let details_rect = body_chunks[1];

        if app.active_pane == ActivePane::ChaptersList {
            app.series_rect = None;
            app.chapters_rect = Some(list_rect);
            render_chapters_list(f, list_rect, app, &theme);
        } else {
            app.series_rect = Some(list_rect);
            app.chapters_rect = None;
            render_series_list(f, list_rect, app, &theme);
        }

        render_details_pane(f, details_rect, app, &theme);

        if has_downloads {
            render_downloads_bar(f, main_chunks[3], app, &theme);
        }

        render_action_bar(f, main_chunks[4], app, &theme, true);
    } else {
        app.tab_rects.clear();
        let has_downloads = !app.download_jobs.is_empty();
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),                                 // Header
                Constraint::Min(12),                                   // Body (Side-by-Side)
                Constraint::Length(if has_downloads { 3 } else { 0 }), // Downloads
                Constraint::Length(1),                                 // Unified Action Bar
            ])
            .split(area);

        render_header(f, main_chunks[0], app, &theme);

        let body = main_chunks[1];
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

        let series_rect = body_chunks[0];
        let chapters_rect = right_chunks[0];
        let details_rect = right_chunks[1];

        app.series_rect = Some(series_rect);
        app.chapters_rect = Some(chapters_rect);

        render_series_list(f, series_rect, app, &theme);
        render_chapters_list(f, chapters_rect, app, &theme);
        render_details_pane(f, details_rect, app, &theme);

        if has_downloads {
            render_downloads_bar(f, main_chunks[2], app, &theme);
        }

        render_action_bar(f, main_chunks[3], app, &theme, false);
    }

    if app.input_mode == crate::app::InputMode::CategoryPicker {
        render_category_modal(f, app, &theme);
    }

    if app.show_help_modal {
        render_help_modal(f, &theme);
    }
}
