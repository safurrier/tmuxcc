use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::AppState;
use crate::git::types::CiStatus;

const ORANGE: Color = Color::Rgb(255, 158, 100);

/// Widget for the PR detail panel (toggleable)
pub struct PrDetailWidget;

impl PrDetailWidget {
    /// Fixed height of the PR detail panel when shown
    pub fn height() -> u16 {
        10
    }

    pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
        let pr = match state.selected_pr() {
            Some(pr) => pr,
            None => return,
        };

        let outer_block = Block::default()
            .title(format!(" PR #{} ", pr.number))
            .title_style(Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(77, 53, 39))); // orange-dim border

        let inner_area = outer_block.inner(area);
        frame.render_widget(outer_block, area);

        // Split into 2 columns
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(inner_area);

        // Left column
        let mut left_lines: Vec<Line> = Vec::new();

        // Title
        let title_display = truncate_str(&pr.title, 40);
        left_lines.push(Line::from(vec![
            Span::styled(
                &title_display,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            if pr.is_draft {
                Span::styled(
                    " [draft]",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::DIM),
                )
            } else {
                Span::raw("")
            },
        ]));

        // Merge status
        let (icon, text, color) = pr.mergeable.display();
        left_lines.push(Line::from(vec![
            Span::styled("Status  ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} {}", icon, text), Style::default().fg(color)),
        ]));

        // Review
        let (rev_icon, rev_color) = pr.review_decision.icon();
        left_lines.push(Line::from(vec![
            Span::styled("Reviews ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} {}", rev_icon, pr.review_decision),
                Style::default().fg(rev_color),
            ),
        ]));

        // Comments + changes
        left_lines.push(Line::from(vec![
            Span::styled("Changes ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("+{}", pr.additions),
                Style::default().fg(Color::Green),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(
                format!("-{}", pr.deletions),
                Style::default().fg(Color::Red),
            ),
            Span::styled(
                format!(" · {} comments", pr.total_comments),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        // Actions hint
        left_lines.push(Line::from(vec![Span::styled(" ", Style::default())]));
        left_lines.push(Line::from(vec![
            Span::styled("[", Style::default().fg(Color::DarkGray)),
            Span::styled("o", Style::default().fg(Color::Yellow)),
            Span::styled("] open  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[", Style::default().fg(Color::DarkGray)),
            Span::styled("c", Style::default().fg(Color::Yellow)),
            Span::styled("] copy url", Style::default().fg(Color::DarkGray)),
        ]));

        let left_paragraph = Paragraph::new(left_lines).wrap(Wrap { trim: false });
        frame.render_widget(left_paragraph, columns[0]);

        // Right column
        let mut right_lines: Vec<Line> = Vec::new();

        // Branch info
        right_lines.push(Line::from(vec![
            Span::styled("Branch  ", Style::default().fg(Color::DarkGray)),
            Span::styled(&pr.head_ref, Style::default().fg(Color::Magenta)),
            Span::styled(" → ", Style::default().fg(Color::DarkGray)),
            Span::styled(&pr.base_ref, Style::default().fg(Color::Magenta)),
        ]));

        // Pipeline dots
        if !pr.checks.is_empty() {
            let mut pipeline_spans = vec![Span::styled(
                "Pipeline",
                Style::default().fg(Color::DarkGray),
            )];
            pipeline_spans.push(Span::raw(" "));
            for check in &pr.checks {
                pipeline_spans.push(Span::styled(
                    "●",
                    Style::default().fg(check.status.dot_color()),
                ));
            }
            right_lines.push(Line::from(pipeline_spans));

            // Failed job names
            let failed: Vec<&str> = pr
                .checks
                .iter()
                .filter(|c| c.status == CiStatus::Failure)
                .map(|c| c.name.as_str())
                .collect();
            if !failed.is_empty() {
                right_lines.push(Line::from(vec![
                    Span::styled("         ", Style::default()),
                    Span::styled("✗ ", Style::default().fg(Color::Red)),
                    Span::styled(failed.join(", "), Style::default().fg(Color::Red)),
                ]));
            }
        }

        // URL
        right_lines.push(Line::from(vec![Span::styled("", Style::default())]));
        let url_display = truncate_str(&pr.url, 45);
        right_lines.push(Line::from(vec![Span::styled(
            url_display,
            Style::default().fg(Color::DarkGray),
        )]));

        let right_paragraph = Paragraph::new(right_lines).wrap(Wrap { trim: false });
        frame.render_widget(right_paragraph, columns[1]);
    }
}

/// Truncate a string to `max_chars` on char boundaries, appending "…" if truncated.
fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars - 1).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentType, MonitoredAgent};
    use crate::git::types::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_state_with_pr() -> AppState {
        let mut state = AppState::new();
        let agent = MonitoredAgent::new(
            "a1".to_string(),
            "main:0.0".to_string(),
            "main".to_string(),
            0,
            "code".to_string(),
            0,
            "/home/user/project".to_string(),
            AgentType::ClaudeCode,
            1000,
        );
        state.agents.root_agents.push(agent);
        state.pr_info.insert(
            "/home/user/project".to_string(),
            PrLookupResult::Found(PrInfo {
                number: 42,
                title: "Add auth middleware".to_string(),
                state: "OPEN".to_string(),
                url: "https://github.com/test/42".to_string(),
                head_ref: "feature/auth".to_string(),
                base_ref: "main".to_string(),
                is_draft: false,
                review_decision: ReviewDecision::Approved,
                mergeable: MergeableState::Mergeable,
                checks: vec![
                    CiCheck {
                        name: "build".to_string(),
                        status: CiStatus::Success,
                    },
                    CiCheck {
                        name: "integration-tests".to_string(),
                        status: CiStatus::Failure,
                    },
                ],
                total_comments: 5,
                additions: 150,
                deletions: 30,
            }),
        );
        state.show_pr_panel = true;
        state
    }

    #[test]
    fn test_detail_panel_renders_title() {
        let state = make_state_with_pr();
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                PrDetailWidget::render(frame, frame.area(), &state);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let mut all_text = String::new();
        for y in 0..12 {
            for x in 0..80 {
                all_text.push_str(buf.cell((x, y)).unwrap().symbol());
            }
        }
        assert!(
            all_text.contains("Add auth middleware"),
            "Expected title in rendered output"
        );
    }

    #[test]
    fn test_detail_panel_shows_failed_job() {
        let state = make_state_with_pr();
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                PrDetailWidget::render(frame, frame.area(), &state);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let mut all_text = String::new();
        for y in 0..12 {
            for x in 0..80 {
                all_text.push_str(buf.cell((x, y)).unwrap().symbol());
            }
        }
        assert!(
            all_text.contains("integration-tests"),
            "Expected failed job name in rendered output"
        );
    }

    #[test]
    fn test_detail_panel_shows_review_status() {
        let state = make_state_with_pr();
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                PrDetailWidget::render(frame, frame.area(), &state);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let mut all_text = String::new();
        for y in 0..12 {
            for x in 0..80 {
                all_text.push_str(buf.cell((x, y)).unwrap().symbol());
            }
        }
        assert!(
            all_text.contains("approved"),
            "Expected review decision in rendered output"
        );
    }

    #[test]
    fn test_truncate_str_with_multibyte_chars() {
        // Should not panic on multibyte UTF-8
        let result = super::truncate_str(
            "日本語のテストタイトル長いタイトルですよこれはとても長い",
            10,
        );
        assert!(result.ends_with('…'));
        assert_eq!(result.chars().count(), 10);

        // Short string not truncated
        let result2 = super::truncate_str("short", 10);
        assert_eq!(result2, "short");

        // ASCII truncation
        let result3 = super::truncate_str("a]bcdefghijklmno", 10);
        assert_eq!(result3.chars().count(), 10);
        assert!(result3.ends_with('…'));
    }
}
