use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::app::{ActivePane, App, AppAction, FilterMode, InputMode};
use crate::ui::theme::Theme;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn get_spinner(tick: usize) -> &'static str {
    SPINNER_FRAMES[tick % SPINNER_FRAMES.len()]
}

pub fn render_header(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let total_series = app.series_list.len();
    let total_downloaded: usize = app
        .series_list
        .iter()
        .map(|s| s.stats.downloaded_chapters)
        .sum();
    let total_completed: usize = app
        .series_list
        .iter()
        .map(|s| s.stats.completed_chapters)
        .sum();

    let lib_path_display = app.config.library_dir.to_string_lossy();

    let mut header_spans = vec![
        Span::styled(
            " Dewey ",
            Style::default()
                .fg(theme.highlight_fg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} Series", total_series),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  •  {} Downloaded", total_downloaded),
            Style::default().fg(theme.muted_fg),
        ),
        Span::styled(
            format!("  •  {} Completed", total_completed),
            Style::default().fg(theme.success),
        ),
        Span::styled(
            format!("  [📁 {}]", lib_path_display),
            Style::default().fg(theme.muted_fg),
        ),
    ];

    if app.is_scanning {
        header_spans.push(Span::styled(
            format!("  {} Scanning library...", get_spinner(app.tick_count)),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let p = Paragraph::new(Line::from(header_spans)).style(Style::default().bg(Color::Reset));
    f.render_widget(p, area);
}

pub fn render_series_list(f: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let is_focused = app.active_pane == ActivePane::SeriesList;
    let border_style = if is_focused {
        theme.active_border()
    } else {
        theme.inactive_border()
    };

    let title_style = if is_focused {
        theme.block_title_focused()
    } else {
        theme.block_title_normal()
    };

    let total_all = app.series_list.len();
    let total_filtered = app.filtered_indices.len();

    let title_text = if app.input_mode == InputMode::SearchInput {
        format!(
            " 🔍 Search: \"{}\"_ [ESC: Cancel, ↵: Done] ",
            app.search_query
        )
    } else if !app.search_query.is_empty()
        || app.filter_mode != FilterMode::All
        || app.type_filter != crate::app::TypeFilter::All
        || app.hidden_filter != crate::app::HiddenFilter::Hide
    {
        let mut filter_desc = Vec::new();
        if !app.search_query.is_empty() {
            filter_desc.push(format!("🔍 \"{}\"", app.search_query));
        }
        if app.type_filter != crate::app::TypeFilter::All {
            filter_desc.push(format!("Type: {}", app.type_filter.label()));
        }
        if app.filter_mode != FilterMode::All {
            filter_desc.push(format!("Status: {}", app.filter_mode.label()));
        }
        match app.hidden_filter {
            crate::app::HiddenFilter::Hide => {}
            crate::app::HiddenFilter::Show => filter_desc.push("👁 Show All".to_string()),
            crate::app::HiddenFilter::Only => filter_desc.push("👁 Only Hidden".to_string()),
        }
        format!(
            " 1. Series ({}/{}) [{}] ",
            total_filtered,
            total_all,
            filter_desc.join(" · ")
        )
    } else {
        format!(" 1. Series Library ({}) ", total_all)
    };

    let title_span = if app.input_mode == InputMode::SearchInput {
        Span::styled(
            title_text,
            Style::default()
                .fg(theme.highlight_fg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(title_text, title_style)
    };

    let items: Vec<ListItem> = if app.filtered_indices.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "   No matching series found",
            Style::default().fg(theme.muted_fg),
        )]))]
    } else {
        app.filtered_indices
            .iter()
            .enumerate()
            .map(|(filtered_pos, &real_idx)| {
                let s = &app.series_list[real_idx];
                let is_selected = filtered_pos == app.selected_series_idx;
                let status_badge = match s.series.status.as_deref() {
                    Some("Completed") => Span::styled(" [CMPL] ", theme.success_badge()),
                    Some("Ongoing") | Some("Continuing") => {
                        Span::styled(" [ONG] ", theme.warning_badge())
                    }
                    _ => Span::raw(" "),
                };

                let hidden_badge = if s.series.is_hidden {
                    Span::styled(" [HIDDEN] ", theme.error_badge())
                } else {
                    Span::raw("")
                };

                let category_badge = if let Some(cat) = &s.series.category {
                    Span::styled(format!(" [{}]", cat), Style::default().fg(theme.accent_alt))
                } else {
                    Span::raw("")
                };

                let read_indicator = if let Some(last_ch) = s.stats.latest_read_chapter {
                    Span::styled(
                        format!("Ch.{:.0} ", last_ch),
                        Style::default().fg(theme.accent),
                    )
                } else {
                    Span::styled("Unread ", Style::default().fg(theme.muted_fg))
                };

                let count_info =
                    format!("{}/{}", s.stats.downloaded_chapters, s.stats.total_chapters);

                let url_indicator = if s.series.fetch_url.is_some() {
                    Span::styled(" 🔗", Style::default().fg(theme.accent_alt))
                } else {
                    Span::raw("")
                };

                let line = Line::from(vec![
                    Span::styled(
                        if is_selected { " ▶ " } else { "   " },
                        if is_selected {
                            theme.title()
                        } else {
                            Style::default()
                        },
                    ),
                    Span::styled(
                        format!("{:<20}", s.series.title),
                        if is_selected {
                            Style::default()
                                .fg(theme.highlight_fg)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme.fg)
                        },
                    ),
                    category_badge,
                    hidden_badge,
                    read_indicator,
                    Span::styled(
                        format!("{:>7}", count_info),
                        Style::default().fg(theme.muted_fg),
                    ),
                    url_indicator,
                    status_badge,
                ]);

                let item_style = if is_selected {
                    theme.selected_item()
                } else {
                    theme.normal_item()
                };

                ListItem::new(line).style(item_style)
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style)
                .title(title_span),
        )
        .highlight_style(theme.selected_item());

    f.render_stateful_widget(list, area, &mut app.series_state);
}

pub fn render_chapters_list(f: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let is_focused = app.active_pane == ActivePane::ChaptersList;
    let border_style = if is_focused {
        theme.active_border()
    } else {
        theme.inactive_border()
    };

    let title_style = if is_focused {
        theme.block_title_focused()
    } else {
        theme.block_title_normal()
    };

    let series_title = app
        .current_series()
        .map(|s| s.series.title.as_str())
        .unwrap_or("No Series Selected");

    let rows: Vec<Row> = app
        .chapters_list
        .iter()
        .enumerate()
        .map(|(idx, chap)| {
            let is_selected = idx == app.selected_chapter_idx;
            let num_str = format!("Ch. {:.1}", chap.chapter.chapter_number);

            let read_status = if chap.is_completed() {
                Span::styled("✓ Completed", theme.success_badge())
            } else if let Some(prog) = &chap.progress {
                if prog.last_page_read > 0 {
                    Span::styled(
                        format!(
                            "Page {}/{}",
                            prog.last_page_read,
                            chap.chapter.page_count.unwrap_or(0)
                        ),
                        Style::default().fg(theme.warning),
                    )
                } else {
                    Span::styled("Unread", theme.muted_item())
                }
            } else {
                Span::styled("Unread", theme.muted_item())
            };

            let file_status = if chap.is_downloaded() {
                Span::styled("✓ Ready", theme.success_badge())
            } else if app.download_jobs.iter().any(|j| {
                app.current_series()
                    .map(|s| s.series.id == j.series_id)
                    .unwrap_or(false)
                    && (j.chapter_number - chap.chapter.chapter_number).abs() < f64::EPSILON
            }) {
                Span::styled(
                    format!("{} Downloading...", get_spinner(app.tick_count)),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("⤓ Fetch Needed", theme.warning_badge())
            };

            let row_style = if is_selected {
                theme.selected_item()
            } else {
                theme.normal_item()
            };

            Row::new(vec![
                Span::raw(if is_selected { "▶" } else { " " }),
                Span::styled(num_str, Style::default().add_modifier(Modifier::BOLD)),
                read_status,
                file_status,
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(2),
        Constraint::Length(12),
        Constraint::Length(18),
        Constraint::Min(15),
    ];

    let header = Row::new(vec!["", "Chapter", "Read Progress", "Download Status"]).style(
        Style::default()
            .fg(theme.muted_fg)
            .add_modifier(Modifier::UNDERLINED),
    );

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style)
                .title(Span::styled(
                    format!(" 2. Chapters — {} ", series_title),
                    title_style,
                )),
        )
        .row_highlight_style(theme.selected_item());

    f.render_stateful_widget(table, area, &mut app.chapters_state);
}

pub fn render_details_pane(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let current_series = app.current_series();

    let content = if let Some(s) = current_series {
        let meta_author = s
            .series
            .metadata_json
            .as_ref()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
            .and_then(|v| {
                let meta = v.get("metadata").unwrap_or(&v);
                meta.get("publisher")
                    .or_else(|| meta.get("author"))
                    .and_then(|a| a.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "Unknown".to_string());

        let meta_genres = s
            .series
            .metadata_json
            .as_ref()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
            .and_then(|v| {
                let meta = v.get("metadata").unwrap_or(&v);
                meta.get("genre").and_then(|g| g.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
            })
            .unwrap_or_else(|| "General".to_string());

        let cover_str = s.series.cover_path.as_deref().unwrap_or("None");
        let mode_display = match s.series.reading_mode() {
            "manga" => "Manga (Horizontal)",
            _ => "Webtoon (Vertical)",
        };
        let fetch_url_str = s
            .series
            .fetch_url
            .as_deref()
            .unwrap_or("Not set (Labrador will resolve)");

        vec![
            Line::from(vec![
                Span::styled("Title:     ", Style::default().fg(theme.muted_fg)),
                Span::styled(
                    &s.series.title,
                    Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Category:  ", Style::default().fg(theme.muted_fg)),
                Span::styled(
                    s.series.category.as_deref().unwrap_or("Uncategorized"),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("   Author: ", Style::default().fg(theme.muted_fg)),
                Span::styled(meta_author, Style::default().fg(theme.fg)),
                Span::styled("   Genres: ", Style::default().fg(theme.muted_fg)),
                Span::styled(meta_genres, Style::default().fg(theme.accent_alt)),
            ]),
            Line::from(vec![
                Span::styled("Status:    ", Style::default().fg(theme.muted_fg)),
                Span::styled(
                    s.series.status.as_deref().unwrap_or("Unknown"),
                    Style::default().fg(theme.warning),
                ),
                Span::styled("   Mode: ", Style::default().fg(theme.muted_fg)),
                Span::styled(mode_display, Style::default().fg(theme.accent)),
                Span::styled("   Cover: ", Style::default().fg(theme.muted_fg)),
                Span::styled(cover_str, Style::default().fg(theme.muted_fg)),
            ]),
            Line::from(vec![
                Span::styled("Fetch URL: ", Style::default().fg(theme.muted_fg)),
                Span::styled(
                    fetch_url_str,
                    if s.series.fetch_url.is_some() {
                        Style::default().fg(theme.accent)
                    } else {
                        Style::default().fg(theme.muted_fg)
                    },
                ),
            ]),
            Line::from(vec![
                Span::styled("Progress:  ", Style::default().fg(theme.muted_fg)),
                Span::styled(
                    format!(
                        " {}/{} Chapters Read ({:.0}%)",
                        s.stats.completed_chapters,
                        s.stats.total_chapters,
                        if s.stats.total_chapters > 0 {
                            (s.stats.completed_chapters as f64 / s.stats.total_chapters as f64)
                                * 100.0
                        } else {
                            0.0
                        }
                    ),
                    Style::default().fg(theme.success),
                ),
            ]),
        ]
    } else {
        vec![Line::from(Span::styled(
            "Select a series to view metadata",
            Style::default().fg(theme.muted_fg),
        ))]
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.inactive_border())
        .title(Span::styled(
            " ℹ Metadata & Source URL ",
            theme.block_title_normal(),
        ));

    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

pub fn render_downloads_bar(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let spinner = get_spinner(app.tick_count);

    let items: Vec<Span> = if app.download_jobs.is_empty() {
        vec![Span::styled(
            " No active background fetch tasks.",
            Style::default().fg(theme.muted_fg),
        )]
    } else {
        let mut spans = vec![Span::styled(
            format!(" {} Labrador Active Tasks: ", spinner),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )];

        for job in &app.download_jobs {
            let elapsed = job.started_at.elapsed().as_secs();
            spans.push(Span::styled(
                format!(
                    " [{}: Ch. {:.1} ({}s)] ",
                    job.series_title, job.chapter_number, elapsed
                ),
                Style::default()
                    .fg(theme.highlight_fg)
                    .bg(theme.highlight_bg),
            ));
        }
        spans
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if !app.download_jobs.is_empty() {
            theme.active_border()
        } else {
            theme.inactive_border()
        })
        .title(Span::styled(
            " 🐕 Labrador Fetching Queue ",
            if !app.download_jobs.is_empty() {
                theme.block_title_focused()
            } else {
                theme.block_title_normal()
            },
        ));

    let paragraph = Paragraph::new(Line::from(items)).block(block);
    f.render_widget(paragraph, area);
}

pub fn render_portrait_tab_bar(f: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    app.tab_rects.clear();

    let tab_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let series_is_active = app.active_pane == ActivePane::SeriesList;
    let series_count = app.series_list.len();
    let series_title = format!(" 📚 1. Series ({}) ", series_count);

    let (series_border, series_style) = if series_is_active {
        (theme.active_border(), theme.block_title_focused())
    } else {
        (theme.inactive_border(), theme.block_title_normal())
    };

    let series_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(series_border)
        .title(Span::styled(series_title, series_style));

    let series_summary = if series_is_active {
        Span::styled(
            app.current_series()
                .map(|s| s.series.title.as_str())
                .unwrap_or("No selection"),
            Style::default()
                .fg(theme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("Tap to view Series", Style::default().fg(theme.muted_fg))
    };

    let p_series = Paragraph::new(Line::from(vec![
        Span::raw(if series_is_active { "▶ " } else { "  " }),
        series_summary,
    ]))
    .alignment(Alignment::Center)
    .block(series_block);
    f.render_widget(p_series, tab_chunks[0]);
    app.tab_rects.push((tab_chunks[0], ActivePane::SeriesList));

    let chapters_is_active = app.active_pane == ActivePane::ChaptersList;
    let chapters_count = app.chapters_list.len();
    let chapters_title = format!(" 📖 2. Chapters ({}) ", chapters_count);

    let (chapters_border, chapters_style) = if chapters_is_active {
        (theme.active_border(), theme.block_title_focused())
    } else {
        (theme.inactive_border(), theme.block_title_normal())
    };

    let chapters_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(chapters_border)
        .title(Span::styled(chapters_title, chapters_style));

    let chapters_summary = if chapters_is_active {
        Span::styled(
            app.current_chapter()
                .map(|c| format!("Ch. {:.1}", c.chapter.chapter_number))
                .unwrap_or_else(|| "No selection".to_string()),
            Style::default()
                .fg(theme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("Tap to view Chapters", Style::default().fg(theme.muted_fg))
    };

    let p_chapters = Paragraph::new(Line::from(vec![
        Span::raw(if chapters_is_active { "▶ " } else { "  " }),
        chapters_summary,
    ]))
    .alignment(Alignment::Center)
    .block(chapters_block);
    f.render_widget(p_chapters, tab_chunks[1]);
    app.tab_rects
        .push((tab_chunks[1], ActivePane::ChaptersList));
}

/// Touch tap targets: records screen rects so taps can be hit-tested.
pub fn render_action_bar(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    theme: &Theme,
    is_portrait: bool,
) {
    app.action_rects.clear();

    if is_portrait && area.height >= 2 {
        let row_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(area);

        let row1_actions = [
            ("📖 Read", "↵", AppAction::Open),
            ("🔍 Find", "/", AppAction::Search),
            ("⚡ Status", "f", AppAction::Filter),
            ("📚 Type", "T", AppAction::CycleType),
            ("👁 Hidden", ".", AppAction::CycleHidden),
            ("🏷 Move", "t", AppAction::TagCategory),
            ("🔒 Hide", "H", AppAction::ToggleHidden),
        ];

        let row2_actions = [
            ("➕ Add", "a", AppAction::AddSeries),
            ("✏ Edit", "e", AppAction::EditSeries),
            ("⬇ Fetch", "d", AppAction::Fetch),
            ("📁 Scan", "s", AppAction::Scan),
            ("✓ Mark", "m", AppAction::MarkRead),
            ("🔄 Mode", "M", AppAction::Mode),
            ("❓ Help", "?", AppAction::Help),
            ("❌ Quit", "q", AppAction::Quit),
        ];

        for (row_idx, (actions, row_area)) in [
            (&row1_actions[..], row_chunks[0]),
            (&row2_actions[..], row_chunks[1]),
        ]
        .iter()
        .enumerate()
        {
            let mut spans = Vec::new();
            let mut current_x = row_area.x;

            for (label, key, action) in *actions {
                let btn_text = format!(" {} [{}] ", label, key);
                let btn_len = btn_text.chars().count() as u16;

                let rect = Rect {
                    x: current_x,
                    y: row_area.y,
                    width: btn_len,
                    height: 1,
                };
                app.action_rects.push((rect, *action));
                current_x += btn_len;

                let fg = if *action == AppAction::Quit {
                    theme.error
                } else {
                    theme.accent
                };
                spans.push(Span::styled(
                    format!(" {} ", label),
                    Style::default().fg(fg),
                ));
                spans.push(Span::styled(
                    format!("[{}] ", key),
                    Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
                ));
            }

            if row_idx == 1 {
                if let Some((msg, is_error, _)) = &app.toast {
                    spans.push(Span::raw(" "));
                    let toast_style = if *is_error {
                        theme.error_badge().add_modifier(Modifier::BOLD)
                    } else {
                        theme.success_badge().add_modifier(Modifier::BOLD)
                    };
                    spans.push(Span::styled(format!("🔔 {}", msg), toast_style));
                }
            }

            let p = Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.bg));
            f.render_widget(p, *row_area);
        }
    } else {
        let actions = [
            ("📖 Read", "↵", AppAction::Open),
            ("➕ Add", "a", AppAction::AddSeries),
            ("✏ Edit", "e", AppAction::EditSeries),
            ("🔍 Find", "/", AppAction::Search),
            ("⚡ Status", "f", AppAction::Filter),
            ("📚 Type", "T", AppAction::CycleType),
            ("👁 Hidden", ".", AppAction::CycleHidden),
            ("🏷 Move", "t", AppAction::TagCategory),
            ("🔒 Hide", "H", AppAction::ToggleHidden),
            ("⬇ Fetch", "d", AppAction::Fetch),
            ("📁 Scan", "s", AppAction::Scan),
            ("✓ Mark", "m", AppAction::MarkRead),
            ("🔄 Mode", "M", AppAction::Mode),
            ("❓ Help", "?", AppAction::Help),
            ("❌ Quit", "q", AppAction::Quit),
        ];

        let mut spans = Vec::new();
        let mut current_x = area.x;

        for (label, key, action) in actions {
            let btn_text = format!(" {} [{}] ", label, key);
            let btn_len = btn_text.chars().count() as u16;

            let rect = Rect {
                x: current_x,
                y: area.y,
                width: btn_len,
                height: 1,
            };
            app.action_rects.push((rect, action));
            current_x += btn_len;

            let fg = if action == AppAction::Quit {
                theme.error
            } else {
                theme.accent
            };
            spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(fg),
            ));
            spans.push(Span::styled(
                format!("[{}] ", key),
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ));
        }

        if let Some((msg, is_error, _)) = &app.toast {
            spans.push(Span::raw(" "));
            let toast_style = if *is_error {
                theme.error_badge().add_modifier(Modifier::BOLD)
            } else {
                theme.success_badge().add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(format!("🔔 {}", msg), toast_style));
        }

        let p = Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.bg));
        f.render_widget(p, area);
    }
}

#[allow(dead_code)]
fn action_key_hint(action: AppAction) -> &'static str {
    match action {
        AppAction::Open => "↵",
        AppAction::AddSeries => "a",
        AppAction::EditSeries => "e",
        AppAction::Fetch => "d",
        AppAction::FetchNext => "D",
        AppAction::Mode => "M",
        AppAction::MarkRead => "m",
        AppAction::Scan => "s",
        AppAction::Reset => "u",
        AppAction::Delete => "x",
        AppAction::SwitchPane => "Tab",
        AppAction::Search => "/",
        AppAction::Filter => "f",
        AppAction::CycleType => "T",
        AppAction::CycleHidden => ".",
        AppAction::TagCategory => "t",
        AppAction::ToggleHidden => "H",
        AppAction::Help => "?",
        AppAction::Quit => "q",
    }
}

pub fn render_help_modal(f: &mut Frame, theme: &Theme) {
    let area = centered_rect(65, 75, f.area());
    f.render_widget(Clear, area);

    let help_text = vec![
        Line::from(vec![
            Span::styled(
                "  /                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Search series library (live fuzzy match)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  f / F                 ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Cycle status filter (All → Unread → Ongoing → Completed)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  T                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Cycle type/medium filter (All → Manhwa → Manga → Comicbook)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  t                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Tag / move series into category folder (e.g. Manga/Action)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  .                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Cycle hidden series filter (Hide → Show All → Only Hidden)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  H                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Toggle hidden tag on selected series (relocates to/from .Other)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  j / Down              ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Next item in active list"),
        ]),
        Line::from(vec![
            Span::styled(
                "  k / Up                ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Previous item in active list"),
        ]),
        Line::from(vec![
            Span::styled(
                "  g / G                 ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Jump to top / bottom of active list"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Tab / h / l / Arrows  ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Switch active pane (Series ↔ Chapters)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  Enter                 ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Launch Continuum reader (or fetch if missing)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  a                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Add new series via Labrador search"),
        ]),
        Line::from(vec![
            Span::styled(
                "  e                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Edit series details & publication status (Ongoing ↔ Complete)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  E                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Quick-toggle publication status (Ongoing ↔ Complete)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  d                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Fetch selected chapter via Labrador"),
        ]),
        Line::from(vec![
            Span::styled(
                "  D                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Fetch next unread chapter via Labrador"),
        ]),
        Line::from(vec![
            Span::styled(
                "  s                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Scan designated library directory for files"),
        ]),
        Line::from(vec![
            Span::styled(
                "  m                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Toggle chapter completed / uncompleted"),
        ]),
        Line::from(vec![
            Span::styled(
                "  M / v                 ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Toggle series reading mode (Webtoon ↔ Manga)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  u                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Mark chapter unread (clear progress)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  x                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Delete selected series (press twice to confirm)"),
        ]),
        Line::from(vec![
            Span::styled(
                "  r                     ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Reload library & database stats"),
        ]),
        Line::from(vec![
            Span::styled(
                "  ? or Esc              ",
                Style::default().fg(theme.warning),
            ),
            Span::raw("Close this help dialog"),
        ]),
        Line::from(vec![
            Span::styled("  q / Ctrl+C            ", Style::default().fg(theme.error)),
            Span::raw("Quit Dewey"),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(theme.active_border())
        .title(" Help & Keybindings ");

    let p = Paragraph::new(help_text)
        .block(block)
        .alignment(Alignment::Left);
    f.render_widget(p, area);
}

pub fn render_category_modal(f: &mut Frame, app: &mut App, theme: &Theme) {
    let screen = f.area();
    let is_portrait = screen.height > screen.width;

    let area = if is_portrait {
        centered_rect(92, 75, screen)
    } else {
        centered_rect(70, 75, screen)
    };
    app.category_modal_rect = Some(area);
    f.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Input box
            Constraint::Min(6),    // Preset categories list
            Constraint::Length(3), // Action touch buttons
            Constraint::Length(1), // Footer hint text
        ])
        .split(area);

    let series_title = app
        .current_series()
        .map(|s| s.series.title.as_str())
        .unwrap_or("Selected Series");

    // 1. Text input for category
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.active_border())
        .title(format!(" 🏷 Move / Tag [{}] Category ", series_title));

    let input_p = Paragraph::new(Line::from(vec![
        Span::styled(
            " Category: ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}_", app.category_input),
            Style::default()
                .fg(theme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(input_block)
    .style(Style::default().bg(theme.highlight_bg));
    f.render_widget(input_p, chunks[0]);

    // 2. Preset / Available category list
    app.category_list_rect = Some(chunks[1]);

    let items: Vec<ListItem> = app
        .available_categories
        .iter()
        .enumerate()
        .map(|(idx, cat)| {
            let is_sel = idx == app.category_selected_idx;
            let prefix = if is_sel { " ▶ " } else { "   " };
            let style = if is_sel {
                Style::default()
                    .fg(theme.highlight_fg)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg)
            };
            ListItem::new(format!("{}{}", prefix, cat)).style(style)
        })
        .collect();

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.inactive_border())
        .title(" Available / Suggested Categories (tap or ↑/↓ to choose) ");

    let list = List::new(items).block(list_block).highlight_style(
        Style::default()
            .fg(theme.highlight_fg)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD),
    );

    f.render_stateful_widget(list, chunks[1], &mut app.category_state);

    // 3. Touch button bar (Confirm, Clear, Cancel)
    let button_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40), // Confirm
            Constraint::Percentage(25), // Clear
            Constraint::Percentage(35), // Cancel
        ])
        .split(chunks[2]);

    app.category_confirm_rect = Some(button_chunks[0]);
    app.category_clear_rect = Some(button_chunks[1]);
    app.category_cancel_rect = Some(button_chunks[2]);

    let confirm_btn = Paragraph::new(Line::from(vec![Span::styled(
        "✔ Confirm [↵]",
        Style::default()
            .fg(theme.highlight_fg)
            .add_modifier(Modifier::BOLD),
    )]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(theme.accent))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent)),
    );
    f.render_widget(confirm_btn, button_chunks[0]);

    let clear_btn = Paragraph::new(Line::from(vec![Span::styled(
        "⌫ Clear",
        Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
    )]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(theme.highlight_bg))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.muted_fg)),
    );
    f.render_widget(clear_btn, button_chunks[1]);

    let cancel_btn = Paragraph::new(Line::from(vec![Span::styled(
        "✖ Cancel [Esc]",
        Style::default()
            .fg(theme.error)
            .add_modifier(Modifier::BOLD),
    )]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(theme.highlight_bg))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.muted_fg)),
    );
    f.render_widget(cancel_btn, button_chunks[2]);

    // 4. Footer hints
    let hints = Paragraph::new(
        "Tap category / button, or type custom folder. Supports nesting like 'Manga/Action'",
    )
    .alignment(Alignment::Center)
    .style(Style::default().fg(theme.muted_fg));
    f.render_widget(hints, chunks[3]);
}

pub fn render_edit_series_modal(f: &mut Frame, app: &mut App, theme: &Theme) {
    let screen = f.area();
    let is_portrait = screen.height > screen.width;

    let area = if is_portrait {
        centered_rect(96, 85, screen)
    } else {
        centered_rect(76, 85, screen)
    };
    app.edit_modal_rect = Some(area);
    f.render_widget(Clear, area);

    let series_title = app
        .current_series()
        .map(|s| s.series.title.as_str())
        .unwrap_or("Selected Series");

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.active_border())
        .title(format!(" ✏ Edit Series [{}] ", series_title));
    f.render_widget(outer_block, area);

    let inner_area = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 0: Status buttons
            Constraint::Length(3), // 1: Title input
            Constraint::Length(3), // 2: Reading mode buttons
            Constraint::Length(3), // 3: Category input
            Constraint::Length(3), // 4: Fetch URL input
            Constraint::Length(3), // 5: Action buttons (Save / Cancel)
            Constraint::Min(1),    // 6: Hint footer
        ])
        .split(inner_area);

    app.edit_status_rects.clear();
    app.edit_mode_rects.clear();

    // 0: Publication Status row
    let status_active = app.edit_field_idx == 0;
    let status_border_style = if status_active {
        theme.active_border()
    } else {
        theme.inactive_border()
    };
    let status_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(status_border_style)
        .title(" 1. Publication Status (Space / ← / → or tap) ");

    let status_inner = status_block.inner(chunks[0]);
    f.render_widget(status_block, chunks[0]);

    let status_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(status_inner);

    for (idx, opt) in crate::app::STATUS_OPTIONS.iter().enumerate() {
        let is_selected = idx == app.edit_status_idx;
        app.edit_status_rects.push((status_cols[idx], idx));

        let icon = match *opt {
            "Completed" => "✔ ",
            "Ongoing" => "⏳ ",
            "Hiatus" => "⏸ ",
            _ => "✖ ",
        };
        let label = format!("{}{}", icon, opt);

        let style = if is_selected {
            if *opt == "Completed" {
                theme.success_badge().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(theme.highlight_fg)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            }
        } else {
            Style::default().fg(theme.fg).bg(theme.highlight_bg)
        };

        let p = Paragraph::new(label)
            .alignment(Alignment::Center)
            .style(style);
        f.render_widget(p, status_cols[idx]);
    }

    // 1: Title input row
    let title_active = app.edit_field_idx == 1;
    let title_border_style = if title_active {
        theme.active_border()
    } else {
        theme.inactive_border()
    };
    app.edit_title_rect = Some(chunks[1]);
    let title_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(title_border_style)
        .title(" 2. Series Title ");

    let title_cursor = if title_active { "_" } else { "" };
    let title_p = Paragraph::new(format!(" {}{}", app.edit_title_input, title_cursor))
        .block(title_block)
        .style(Style::default().fg(theme.fg));
    f.render_widget(title_p, chunks[1]);

    // 2: Reading Mode row
    let mode_active = app.edit_field_idx == 2;
    let mode_border_style = if mode_active {
        theme.active_border()
    } else {
        theme.inactive_border()
    };
    let mode_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(mode_border_style)
        .title(" 3. Reading Mode (Space / ← / → or tap) ");

    let mode_inner = mode_block.inner(chunks[2]);
    f.render_widget(mode_block, chunks[2]);

    let mode_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(mode_inner);

    let mode_labels = [
        "📖 Manga (Horizontal Continuous)",
        "📜 Webtoon (Vertical Continuous)",
    ];
    for (idx, label) in mode_labels.iter().enumerate() {
        let is_selected = idx == app.edit_reading_mode_idx;
        app.edit_mode_rects.push((mode_cols[idx], idx));

        let style = if is_selected {
            Style::default()
                .fg(theme.highlight_fg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg).bg(theme.highlight_bg)
        };

        let p = Paragraph::new(*label)
            .alignment(Alignment::Center)
            .style(style);
        f.render_widget(p, mode_cols[idx]);
    }

    // 3: Category input row
    let cat_active = app.edit_field_idx == 3;
    let cat_border_style = if cat_active {
        theme.active_border()
    } else {
        theme.inactive_border()
    };
    app.edit_category_rect = Some(chunks[3]);
    let cat_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(cat_border_style)
        .title(" 4. Category Folder (e.g. Manga, Manhwa, Action) ");

    let cat_cursor = if cat_active { "_" } else { "" };
    let cat_p = Paragraph::new(format!(" {}{}", app.edit_category_input, cat_cursor))
        .block(cat_block)
        .style(Style::default().fg(theme.fg));
    f.render_widget(cat_p, chunks[3]);

    // 4: Fetch URL input row
    let url_active = app.edit_field_idx == 4;
    let url_border_style = if url_active {
        theme.active_border()
    } else {
        theme.inactive_border()
    };
    app.edit_fetch_url_rect = Some(chunks[4]);
    let url_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(url_border_style)
        .title(" 5. Source Fetch URL (Labrador provider link) ");

    let url_cursor = if url_active { "_" } else { "" };
    let url_p = Paragraph::new(format!(" {}{}", app.edit_fetch_url_input, url_cursor))
        .block(url_block)
        .style(Style::default().fg(theme.fg));
    f.render_widget(url_p, chunks[4]);

    // 5: Save / Cancel buttons row
    let btn_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[5]);

    app.edit_save_rect = Some(btn_cols[0]);
    app.edit_cancel_rect = Some(btn_cols[1]);

    let save_active = app.edit_field_idx == 5;
    let save_border = if save_active {
        theme.active_border()
    } else {
        Style::default().fg(theme.accent)
    };
    let save_btn = Paragraph::new(Line::from(vec![Span::styled(
        "💾 Save Changes [↵]",
        Style::default()
            .fg(theme.highlight_fg)
            .add_modifier(Modifier::BOLD),
    )]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(theme.accent))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(save_border),
    );
    f.render_widget(save_btn, btn_cols[0]);

    let cancel_btn = Paragraph::new(Line::from(vec![Span::styled(
        "✖ Cancel [Esc]",
        Style::default()
            .fg(theme.error)
            .add_modifier(Modifier::BOLD),
    )]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(theme.highlight_bg))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.muted_fg)),
    );
    f.render_widget(cancel_btn, btn_cols[1]);

    // 6: Footer instructions
    let hints = Paragraph::new(
        "Tab / ↓: Next Field  •  Shift+Tab / ↑: Prev Field  •  Space: Toggle Option  •  Enter: Save  •  Esc: Cancel",
    )
    .alignment(Alignment::Center)
    .style(Style::default().fg(theme.muted_fg));
    f.render_widget(hints, chunks[6]);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
