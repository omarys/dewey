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
    } else if !app.search_query.is_empty() || app.filter_mode != FilterMode::All {
        let mut filter_desc = Vec::new();
        if !app.search_query.is_empty() {
            filter_desc.push(format!("🔍 \"{}\"", app.search_query));
        }
        if app.filter_mode != FilterMode::All {
            filter_desc.push(format!("Filter: {}", app.filter_mode.label()));
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
                Span::styled("Author:    ", Style::default().fg(theme.muted_fg)),
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
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        let row1_actions = [
            ("📖 Read", AppAction::Open),
            ("⬇ Fetch", AppAction::Fetch),
            ("⏭ Next", AppAction::FetchNext),
            ("🔍 Find", AppAction::Search),
            ("⚡ Filter", AppAction::Filter),
        ];

        let row2_actions = [
            ("🔄 Mode", AppAction::Mode),
            ("✓ Mark", AppAction::MarkRead),
            ("📁 Scan", AppAction::Scan),
            ("🗑 Del", AppAction::Delete),
            ("❌ Quit", AppAction::Quit),
        ];

        for (actions, row_area) in [
            (&row1_actions, row_chunks[0]),
            (&row2_actions, row_chunks[1]),
        ] {
            let n = actions.len() as u16;
            let cells = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![Constraint::Percentage(100 / n); n as usize])
                .split(row_area);

            for (cell, (label, action)) in cells.iter().zip(actions.iter()) {
                app.action_rects.push((*cell, *action));
                let key = action_key(*action);
                let text = format!("{} [{}]", label, key);
                let fg_color = if *action == AppAction::Quit {
                    theme.error
                } else {
                    theme.accent
                };
                let p = Paragraph::new(Line::from(vec![Span::styled(
                    text,
                    Style::default().fg(fg_color).add_modifier(Modifier::BOLD),
                )]))
                .alignment(Alignment::Center)
                .style(Style::default().bg(theme.highlight_bg));
                f.render_widget(p, *cell);
            }
        }
    } else {
        let actions = [
            ("📖 Read", AppAction::Open),
            ("⬇ Fetch", AppAction::Fetch),
            ("⏭ Next", AppAction::FetchNext),
            ("🔍 Find", AppAction::Search),
            ("⚡ Filter", AppAction::Filter),
            ("🔄 Mode", AppAction::Mode),
            ("✓ Mark", AppAction::MarkRead),
            ("📁 Scan", AppAction::Scan),
            ("🗑 Del", AppAction::Delete),
            ("❌ Quit", AppAction::Quit),
        ];
        let n = actions.len() as u16;
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Percentage(100 / n); n as usize])
            .split(area);

        for (cell, (label, action)) in cells.iter().zip(actions.iter()) {
            app.action_rects.push((*cell, *action));
            let text = format!("{} {}", label, action_key(*action));
            let fg_color = if *action == AppAction::Quit {
                theme.error
            } else {
                theme.accent
            };
            let span = Span::styled(text, Style::default().fg(fg_color));
            let p = Paragraph::new(Line::from(vec![span]))
                .alignment(Alignment::Center)
                .style(Style::default().bg(theme.bg));
            f.render_widget(p, *cell);
        }
    }
}

/// The keyboard shortcut behind each action.
fn action_key(action: AppAction) -> &'static str {
    match action {
        AppAction::Open => "↵",
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
        AppAction::Quit => "q",
    }
}

pub fn render_footer(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut spans = vec![
        Span::styled(
            " [/] ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Search  ", Style::default().fg(theme.fg)),
        Span::styled(
            " [f] ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("Filter: {}  ", app.filter_mode.label()),
            Style::default().fg(theme.fg),
        ),
        Span::styled(
            " [Enter] ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Read  ", Style::default().fg(theme.fg)),
        Span::styled(
            " [d] ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Fetch  ", Style::default().fg(theme.fg)),
        Span::styled(
            " [s] ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Scan  ", Style::default().fg(theme.fg)),
        Span::styled(
            " [m] ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Mark Read  ", Style::default().fg(theme.fg)),
        Span::styled(
            " [M] ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Mode  ", Style::default().fg(theme.fg)),
        Span::styled(
            " [Tab] ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Pane  ", Style::default().fg(theme.fg)),
        Span::styled(
            " [?] ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Help  ", Style::default().fg(theme.fg)),
        Span::styled(
            " [q] ",
            Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Quit", Style::default().fg(theme.fg)),
    ];

    if let Some((msg, is_error, _)) = &app.toast {
        spans.push(Span::raw("   |   "));
        let toast_style = if *is_error {
            theme.error_badge().add_modifier(Modifier::BOLD)
        } else {
            theme.success_badge().add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(format!("🔔 {}", msg), toast_style));
    }

    let p = Paragraph::new(Line::from(spans))
        .alignment(Alignment::Left)
        .style(Style::default().bg(theme.bg));
    f.render_widget(p, area);
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
