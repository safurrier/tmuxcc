mod agent_tree;
mod footer;
mod header;
mod help;
mod input;
mod pane_preview;
pub mod pr_detail;
pub mod pr_status_bar;
mod subagent_log;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::BorderType,
};

/// Returns (border_type, border_style) for a panel based on focus.
pub fn panel_border(focused: bool, focus_color: Color) -> (BorderType, Style) {
    if focused {
        (
            BorderType::Double,
            Style::default()
                .fg(focus_color)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (BorderType::Rounded, Style::default().fg(Color::Gray))
    }
}

/// Returns a styled title Line for a panel.
pub fn panel_title(text: &str, focused: bool, focus_color: Color) -> Line<'static> {
    if focused {
        Line::from(Span::styled(
            text.to_string(),
            Style::default()
                .fg(Color::Black)
                .bg(focus_color)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(text.to_string())
    }
}

pub use agent_tree::AgentTreeWidget;
pub use footer::{FooterButton, FooterWidget};
pub use header::HeaderWidget;
pub use help::HelpWidget;
pub use input::InputWidget;
pub use pane_preview::PanePreviewWidget;
pub use pr_detail::PrDetailWidget;
pub use pr_status_bar::PrStatusBarWidget;
pub use subagent_log::SubagentLogWidget;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_border_focused() {
        let (border_type, style) = panel_border(true, Color::Cyan);
        assert_eq!(border_type, BorderType::Double);
        // Style should have bold modifier
        assert!(style.add_modifier == Modifier::BOLD);
    }

    #[test]
    fn test_panel_border_unfocused() {
        let (border_type, _style) = panel_border(false, Color::Cyan);
        assert_eq!(border_type, BorderType::Rounded);
    }

    #[test]
    fn test_panel_title_focused_has_content() {
        let line = panel_title(" Test ", true, Color::Cyan);
        // Should have exactly one span with styled content
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, " Test ");
    }

    #[test]
    fn test_panel_title_unfocused_plain() {
        let line = panel_title(" Test ", false, Color::Cyan);
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, " Test ");
    }
}
