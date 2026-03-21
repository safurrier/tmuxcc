use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::AppState;
use crate::git::types::PrInfo;

/// Widget for the PR status bar (1-line summary)
pub struct PrStatusBarWidget;

impl PrStatusBarWidget {
    /// Returns the height needed: 1 if PR exists for selected agent, 0 otherwise
    pub fn height(state: &AppState) -> u16 {
        if state.selected_pr().is_some() {
            1
        } else {
            0
        }
    }

    pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
        let pr = match state.selected_pr() {
            Some(pr) => pr,
            None => return,
        };

        let agent = match state.selected_agent() {
            Some(a) => a,
            None => return,
        };

        let mut spans = Vec::new();

        // Short path
        spans.push(Span::styled(
            format!(" {}", agent.short_path()),
            Style::default().fg(Color::DarkGray),
        ));

        spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));

        // Branch → base
        spans.push(Span::styled(
            &pr.head_ref,
            Style::default().fg(Color::Magenta),
        ));
        spans.push(Span::styled(" → ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            &pr.base_ref,
            Style::default().fg(Color::Magenta),
        ));

        spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));

        // PR number
        spans.push(Span::styled(
            format!("PR #{}", pr.number),
            Style::default()
                .fg(Color::Rgb(255, 158, 100)) // orange
                .add_modifier(Modifier::BOLD),
        ));

        spans.push(Span::raw(" "));

        // Merge status
        let (icon, text, color) = pr.mergeable.display();
        spans.push(Span::styled(
            format!("{} {}", icon, text),
            Style::default().fg(color),
        ));

        spans.push(Span::raw("  "));

        // CI dots
        render_ci_dots(pr, &mut spans);

        // Draft indicator
        if pr.is_draft {
            spans.push(Span::styled(
                " [draft]",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::DIM),
            ));
        }

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line).style(Style::default().bg(Color::Rgb(30, 32, 48)));

        frame.render_widget(paragraph, area);
    }
}

/// Render CI check dots into a span list
pub fn render_ci_dots(pr: &PrInfo, spans: &mut Vec<Span<'_>>) {
    for check in &pr.checks {
        spans.push(Span::styled(
            "●",
            Style::default().fg(check.status.dot_color()),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentType, MonitoredAgent};
    use crate::git::types::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_pr() -> PrInfo {
        PrInfo {
            number: 42,
            title: "Add auth".to_string(),
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
                    name: "test".to_string(),
                    status: CiStatus::Failure,
                },
            ],
            total_comments: 5,
            additions: 100,
            deletions: 20,
        }
    }

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
            PrLookupResult::Found(make_pr()),
        );
        state
    }

    #[test]
    fn test_status_bar_height_with_pr() {
        let state = make_state_with_pr();
        assert_eq!(PrStatusBarWidget::height(&state), 1);
    }

    #[test]
    fn test_status_bar_height_without_pr() {
        let state = AppState::new();
        assert_eq!(PrStatusBarWidget::height(&state), 0);
    }

    #[test]
    fn test_status_bar_renders_pr_number() {
        let state = make_state_with_pr();
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                PrStatusBarWidget::render(frame, frame.area(), &state);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let content: String = (0..80)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(
            content.contains("PR #42"),
            "Expected 'PR #42' in: {}",
            content
        );
        assert!(
            content.contains("mergeable"),
            "Expected 'mergeable' in: {}",
            content
        );
    }
}
