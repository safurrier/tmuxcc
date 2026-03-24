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
