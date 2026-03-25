use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
        Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use crate::app::{
    generate_flash_labels, sort_agents, Action, AppState, Config, FlashMode, FlashTarget,
    TreeCursor,
};
use crate::monitor::{MonitorTask, SystemStatsCollector};
use crate::notifications::{AgentNotificationInfo, Notifier};
use crate::parsers::ParserRegistry;
use crate::tmux::TmuxClient;

use super::components::{
    AgentTreeWidget, FooterWidget, HeaderWidget, HelpWidget, InputWidget, PanePreviewWidget,
    PrDetailWidget, PrStatusBarWidget, SubagentLogWidget,
};
use super::Layout;

/// Runs the main application loop
pub async fn run_app(config: Config) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableFocusChange
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initialize state
    tracing::info!(
        poll_ms = config.poll_interval_ms,
        popup = config.popup,
        pr_enabled = config.pr_enabled,
        "starting tmuxcc"
    );
    let mut state = AppState::new();
    state.popup_mode = config.popup;
    state.notifications_enabled = config.notifications.enabled;

    // Create tmux client and parser registry
    let tmux_client = Arc::new(TmuxClient::with_capture_lines(config.capture_lines));
    let parser_registry = Arc::new(ParserRegistry::new());

    // Check if tmux is available
    if !tmux_client.is_available() {
        state.set_error("tmux is not running".to_string());
    }

    // Create channel for monitor updates
    let (tx, mut rx) = mpsc::channel(32);

    // Start monitor task
    let monitor = MonitorTask::new(
        tmux_client.clone(),
        parser_registry.clone(),
        tx,
        Duration::from_millis(config.poll_interval_ms),
    );
    let monitor_handle = tokio::spawn(async move {
        monitor.run().await;
    });

    // Create system stats collector
    let mut system_stats = SystemStatsCollector::new();

    // Create notifier
    let mut notifier = Notifier::new(&config.notifications);

    // Create PR monitor task
    let (pr_tx, mut pr_rx) = mpsc::channel(16);
    let (paths_tx, paths_rx) = tokio::sync::watch::channel(Vec::new());
    let pr_monitor_handle = if config.pr_enabled {
        let pr_monitor = crate::git::monitor::PrMonitorTask::new(
            pr_tx,
            Duration::from_millis(config.pr_poll_interval_ms),
            paths_rx,
        );
        Some(tokio::spawn(async move { pr_monitor.run().await }))
    } else {
        None
    };

    // Main loop
    let result = run_loop(
        &mut terminal,
        &mut state,
        &mut rx,
        &mut pr_rx,
        &paths_tx,
        &tmux_client,
        &mut system_stats,
        &mut notifier,
    )
    .await;

    // Cleanup
    monitor_handle.abort();
    if let Some(h) = pr_monitor_handle {
        h.abort();
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableFocusChange
    )?;
    terminal.show_cursor()?;

    result
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    rx: &mut mpsc::Receiver<crate::monitor::MonitorUpdate>,
    pr_rx: &mut mpsc::Receiver<crate::git::monitor::PrMonitorUpdate>,
    paths_tx: &tokio::sync::watch::Sender<Vec<String>>,
    tmux_client: &TmuxClient,
    system_stats: &mut SystemStatsCollector,
    notifier: &mut Notifier,
) -> Result<()> {
    loop {
        // Advance animation tick
        state.tick();

        // Update system stats
        system_stats.refresh();
        state.system_stats = system_stats.stats().clone();

        // Draw UI
        terminal.draw(|frame| {
            let size = frame.area();
            let main_chunks = Layout::main_layout(size);

            // Header
            HeaderWidget::render(frame, main_chunks[0], state);

            // Always show input widget at bottom of right column
            let input_height = InputWidget::calculate_height(state.get_input(), 6);

            if state.show_subagent_log {
                // With subagent log: sidebar | summary+preview+input | subagent_log
                let (left, preview, subagent_log) =
                    Layout::content_layout_with_log(main_chunks[1], state.sidebar_width);
                AgentTreeWidget::render(frame, left, state);

                // Split preview area for summary, preview, and input
                let preview_chunks = ratatui::layout::Layout::default()
                    .direction(ratatui::layout::Direction::Vertical)
                    .constraints([
                        ratatui::layout::Constraint::Length(15),
                        ratatui::layout::Constraint::Min(5),
                        ratatui::layout::Constraint::Length(input_height + 2),
                    ])
                    .split(preview);
                PanePreviewWidget::render_summary(frame, preview_chunks[0], state);
                PanePreviewWidget::render_detailed(frame, preview_chunks[1], state);
                InputWidget::render(frame, preview_chunks[2], state);
                SubagentLogWidget::render(frame, subagent_log, state);
            } else {
                // Normal: sidebar | summary+pr_status+pr_detail+preview+input
                let pr_status_h = PrStatusBarWidget::height(state);
                let pr_detail_h = if state.show_pr_panel && state.selected_pr().is_some() {
                    PrDetailWidget::height()
                } else {
                    0
                };
                let (left, summary, pr_status, pr_detail, preview, input_area) =
                    Layout::content_layout_with_input(
                        main_chunks[1],
                        state.sidebar_width,
                        input_height,
                        state.show_summary_detail,
                        pr_status_h,
                        pr_detail_h,
                    );
                AgentTreeWidget::render(frame, left, state);
                if state.show_summary_detail {
                    PanePreviewWidget::render_summary(frame, summary, state);
                }
                PrStatusBarWidget::render(frame, pr_status, state);
                if pr_detail_h > 0 {
                    PrDetailWidget::render(frame, pr_detail, state);
                }
                PanePreviewWidget::render_detailed(frame, preview, state);
                InputWidget::render(frame, input_area, state);
            }

            // Footer
            FooterWidget::render(frame, main_chunks[2], state);

            // Help overlay
            if state.show_help {
                HelpWidget::render(frame, size, state.help_scroll);
            }
        })?;

        // Handle events with short timeout for responsive UI (~60fps)
        let timeout = Duration::from_millis(16);

        tokio::select! {
            // Handle monitor updates
            Some(update) = rx.recv() => {
                // Diff agent statuses before overwriting to fire notifications
                {
                    use std::collections::HashMap;
                    let old_statuses: HashMap<&str, &crate::agents::AgentStatus> = state
                        .agents
                        .root_agents
                        .iter()
                        .map(|a| (a.id.as_str(), &a.status))
                        .collect();

                    let selected_id = state.selected_agent().map(|a| a.id.clone());

                    for new_agent in &update.agents.root_agents {
                        if let Some(old_status) = old_statuses.get(new_agent.id.as_str()) {
                            let is_selected = selected_id.as_deref() == Some(&new_agent.id);
                            let info = AgentNotificationInfo {
                                agent_id: new_agent.id.clone(),
                                agent_label: format!(
                                    "{} \u{00b7} {}",
                                    new_agent.agent_type.short_name(),
                                    new_agent.short_path()
                                ),
                                session: new_agent.session.clone(),
                                window: new_agent.window,
                                window_name: new_agent.window_name.clone(),
                                target: new_agent.target.clone(),
                                is_active_pane: new_agent.is_active_pane,
                            };
                            notifier.check_and_notify(
                                &info,
                                old_status,
                                &new_agent.status,
                                is_selected,
                            );
                        }
                    }
                }

                state.agents = update.agents;
                state.all_sessions = update.all_sessions;
                // Apply current sort mode to the incoming agents
                sort_agents(&mut state.agents.root_agents, state.sort_mode);
                // Clamp cursor to valid range
                state.clamp_cursor();
                // Clean up invalid selections
                let max_idx = state.agents.root_agents.len();
                state.selected_agents.retain(|&idx| idx < max_idx);

                // Push current agent paths to PR monitor
                let agent_paths: Vec<String> = state.agents.root_agents.iter()
                    .map(|a| a.path.clone())
                    .collect();
                let _ = paths_tx.send(agent_paths);
            }

            // Handle PR monitor updates
            Some(pr_update) = pr_rx.recv() => {
                state.pr_info = pr_update.results;
                state.handle_pr_auto_open();
            }

            // Handle keyboard and mouse events
            _ = tokio::time::sleep(timeout) => {
                // Process all pending events to avoid input lag
                while event::poll(Duration::from_millis(0))? {
                    let event = event::read()?;

                    // Handle focus events for notification suppression
                    if let Event::FocusGained = event {
                        notifier.app_focused = true;
                        continue;
                    }
                    if let Event::FocusLost = event {
                        notifier.app_focused = false;
                        continue;
                    }

                    // Handle mouse events
                    if let Event::Mouse(mouse) = event {
                        let size = terminal.size()?;
                        let area = ratatui::layout::Rect::new(0, 0, size.width, size.height);
                        let main_chunks = Layout::main_layout(area);
                        let footer_area = main_chunks[2];
                        let (sidebar, _, _, _, _, input_area) = Layout::content_layout_with_input(
                            main_chunks[1], state.sidebar_width, 3, state.show_summary_detail,
                            PrStatusBarWidget::height(state),
                            if state.show_pr_panel && state.selected_pr().is_some() { PrDetailWidget::height() } else { 0 },
                        );

                        match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                let x = mouse.column;
                                let y = mouse.row;

                                // Check footer button clicks first
                                if let Some(button) = FooterWidget::hit_test(x, y, footer_area, state) {
                                    use super::components::FooterButton;
                                    match button {
                                        FooterButton::Approve => {
                                            let indices = state.get_operation_indices();
                                            for idx in indices {
                                                if let Some(agent) = state.agents.get_agent(idx) {
                                                    if agent.status.needs_attention() {
                                                        let target = agent.target.clone();
                                                        let _ = tmux_client.send_keys(&target, "y");
                                                        let _ = tmux_client.send_keys(&target, "Enter");
                                                    }
                                                }
                                            }
                                            state.clear_selection();
                                        }
                                        FooterButton::Reject => {
                                            let indices = state.get_operation_indices();
                                            for idx in indices {
                                                if let Some(agent) = state.agents.get_agent(idx) {
                                                    if agent.status.needs_attention() {
                                                        let target = agent.target.clone();
                                                        let _ = tmux_client.send_keys(&target, "n");
                                                        let _ = tmux_client.send_keys(&target, "Enter");
                                                    }
                                                }
                                            }
                                            state.clear_selection();
                                        }
                                        FooterButton::ApproveAll => {
                                            for agent in &state.agents.root_agents {
                                                if agent.status.needs_attention() {
                                                    let _ = tmux_client.send_keys(&agent.target, "y");
                                                    let _ = tmux_client.send_keys(&agent.target, "Enter");
                                                }
                                            }
                                        }
                                        FooterButton::ToggleSelect => {
                                            state.toggle_selection();
                                        }
                                        FooterButton::Focus => {
                                            if let Some(target) = state.selected_pane_target() {
                                                let _ = tmux_client.focus_pane(&target);
                                            }
                                        }
                                        FooterButton::Help => {
                                            state.toggle_help();
                                        }
                                        FooterButton::Quit => {
                                            state.should_quit = true;
                                        }
                                    }
                                }
                                // Check if click is in sidebar - try to select agent
                                else if x >= sidebar.x && x < sidebar.x + sidebar.width
                                    && y >= sidebar.y && y < sidebar.y + sidebar.height
                                {
                                    state.focus_sidebar();
                                    // Calculate which agent was clicked based on row
                                    let rel_y = (y - sidebar.y).saturating_sub(1) as usize;
                                    let agents_count = state.agents.root_agents.len();
                                    if agents_count > 0 {
                                        // Estimate ~4 lines per agent (header + info + status)
                                        let estimated_idx = rel_y / 4;
                                        if estimated_idx < agents_count {
                                            state.select_agent(estimated_idx);
                                        }
                                    }
                                }
                                // Check if click is in input area
                                else if x >= input_area.x && x < input_area.x + input_area.width
                                    && y >= input_area.y && y < input_area.y + input_area.height
                                {
                                    state.focus_input();
                                }
                            }
                            MouseEventKind::ScrollUp => {
                                state.select_prev();
                            }
                            MouseEventKind::ScrollDown => {
                                state.select_next();
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // Handle keyboard events
                    if let Event::Key(key) = event {
                        let action = map_key_to_action(key.code, key.modifiers, state);

                        // Clear pending_kill on any action that isn't KillPane
                        if !matches!(action, Action::KillPanePending | Action::KillPane) {
                            state.pending_kill = None;
                        }

                        // Clear rename_mode on non-rename/non-input actions
                        if state.rename_mode.is_some()
                            && !matches!(
                                action,
                                Action::RenameStart
                                    | Action::RenameExecute(_)
                                    | Action::RenameCancel
                                    | Action::InputChar(_)
                                    | Action::InputBackspace
                                    | Action::CursorLeft
                                    | Action::CursorRight
                                    | Action::CursorHome
                                    | Action::CursorEnd
                            )
                        {
                            state.rename_mode = None;
                            state.take_input();
                        }

                        match action {
                            Action::Quit => {
                                state.should_quit = true;
                            }
                            Action::NextAgent => {
                                state.select_next();
                            }
                            Action::PrevAgent => {
                                state.select_prev();
                            }
                            Action::ToggleSelection => {
                                state.toggle_selection();
                            }
                            Action::SelectAll => {
                                state.select_all();
                            }
                            Action::ClearSelection => {
                                state.clear_selection();
                            }
                            Action::Approve => {
                                let indices = state.get_operation_indices();
                                for idx in indices {
                                    if let Some(agent) = state.agents.get_agent(idx) {
                                        if agent.status.needs_attention() {
                                            let target = agent.target.clone();
                                            if let Err(e) = tmux_client.send_keys(&target, "y") {
                                                state.set_error(format!("Failed to approve: {}", e));
                                                break;
                                            }
                                            if let Err(e) = tmux_client.send_keys(&target, "Enter") {
                                                state.set_error(format!("Failed to send Enter: {}", e));
                                                break;
                                            }
                                        }
                                    }
                                }
                                state.clear_selection();
                            }
                            Action::Reject => {
                                let indices = state.get_operation_indices();
                                for idx in indices {
                                    if let Some(agent) = state.agents.get_agent(idx) {
                                        if agent.status.needs_attention() {
                                            let target = agent.target.clone();
                                            if let Err(e) = tmux_client.send_keys(&target, "n") {
                                                state.set_error(format!("Failed to reject: {}", e));
                                                break;
                                            }
                                            if let Err(e) = tmux_client.send_keys(&target, "Enter") {
                                                state.set_error(format!("Failed to send Enter: {}", e));
                                                break;
                                            }
                                        }
                                    }
                                }
                                state.clear_selection();
                            }
                            Action::ApproveAll => {
                                for agent in &state.agents.root_agents {
                                    if agent.status.needs_attention() {
                                        if let Err(e) = tmux_client.send_keys(&agent.target, "y") {
                                            state.set_error(format!("Failed to approve {}: {}", agent.target, e));
                                            break;
                                        }
                                        if let Err(e) = tmux_client.send_keys(&agent.target, "Enter") {
                                            state.set_error(format!("Failed to send Enter to {}: {}", agent.target, e));
                                            break;
                                        }
                                    }
                                }
                            }
                            Action::FocusPane => {
                                if let Some(target) = state.selected_pane_target() {
                                    if let Err(e) = tmux_client.focus_pane(&target) {
                                        state.set_error(format!("Failed to focus: {}", e));
                                    } else if state.popup_mode {
                                        tracing::info!("popup mode: quitting after focus");
                                        state.should_quit = true;
                                    }
                                }
                            }
                            Action::ToggleCollapse => {
                                state.toggle_collapse();
                            }
                            Action::CollapseAll => {
                                state.collapse_all();
                            }
                            Action::ExpandAll => {
                                state.expand_all();
                            }
                            Action::SearchStart => {
                                state.pre_search_cursor = Some(state.cursor.clone());
                                state.search_query = Some(String::new());
                            }
                            Action::SearchInput(c) => {
                                if let Some(ref mut query) = state.search_query {
                                    query.push(c);
                                }
                                state.apply_search();
                            }
                            Action::SearchBackspace => {
                                if let Some(ref mut query) = state.search_query {
                                    query.pop();
                                }
                                state.apply_search();
                            }
                            Action::SearchNext => {
                                state.search_next();
                            }
                            Action::SearchPrev => {
                                state.search_prev();
                            }
                            Action::SearchConfirm => {
                                state.search_query = None;
                                state.pre_search_cursor = None;
                                // If on a session header, expand and move to first child
                                if matches!(state.cursor, TreeCursor::Session(_)) {
                                    if let Some(session) =
                                        state.selected_session().map(|s| s.to_string())
                                    {
                                        state.collapsed_sessions.remove(&session);
                                    }
                                    state.select_next(); // move to first child
                                } else if let Some(target) = state.selected_pane_target() {
                                    if let Err(e) = tmux_client.focus_pane(&target) {
                                        state.set_error(format!("Failed to focus: {}", e));
                                    } else if state.popup_mode {
                                        tracing::info!(
                                            "popup mode: quitting after search confirm"
                                        );
                                        state.should_quit = true;
                                    }
                                }
                            }
                            Action::SearchCancel => {
                                state.search_query = None;
                                if let Some(cursor) = state.pre_search_cursor.take() {
                                    state.cursor = cursor;
                                }
                            }
                            Action::ToggleNotifications => {
                                notifier.enabled = !notifier.enabled;
                                state.notifications_enabled = notifier.enabled;
                            }
                            Action::ToggleHideNonAgentSessions => {
                                state.hide_non_agent_sessions = !state.hide_non_agent_sessions;
                            }
                            Action::ToggleHideNonAgentPanes => {
                                state.hide_non_agent_panes = !state.hide_non_agent_panes;
                            }
                            Action::CycleSortMode => {
                                state.cycle_sort_mode();
                            }
                            Action::ToggleSubagentLog => {
                                state.toggle_subagent_log();
                            }
                            Action::ToggleSummaryDetail => {
                                state.toggle_summary_detail();
                            }
                            Action::TogglePrPanel => {
                                state.toggle_pr_panel();
                            }
                            Action::OpenPrUrl => {
                                if let Some(pr) = state.selected_pr() {
                                    let url = pr.url.clone();
                                    if let Err(e) = crate::git::open_url(&url) {
                                        state.set_error(format!("Failed to open PR: {}", e));
                                    }
                                }
                            }
                            Action::CopyPrUrl => {
                                if let Some(pr) = state.selected_pr() {
                                    let url = pr.url.clone();
                                    if let Err(e) = crate::git::copy_to_clipboard(&url) {
                                        state.set_error(format!("Failed to copy: {}", e));
                                    }
                                }
                            }
                            Action::Refresh => {
                                state.clear_error();
                            }
                            Action::ShowHelp => {
                                state.toggle_help();
                            }
                            Action::HideHelp => {
                                state.show_help = false;
                            }
                            Action::FocusInput => {
                                state.search_query = None;
                                state.pre_search_cursor = None;
                                state.focus_input();
                            }
                            Action::FocusSidebar => {
                                state.focus_sidebar();
                            }
                            Action::FocusPreview => {
                                state.search_query = None;
                                state.pre_search_cursor = None;
                                state.focus_preview();
                            }
                            Action::CycleFocus => {
                                state.search_query = None;
                                state.pre_search_cursor = None;
                                state.toggle_focus();
                            }
                            Action::ClearInput => {
                                state.take_input();
                            }
                            Action::InputChar(c) => {
                                state.input_char(c);
                            }
                            Action::InputNewline => {
                                state.input_newline();
                            }
                            Action::InputBackspace => {
                                state.input_backspace();
                            }
                            Action::CursorLeft => {
                                state.cursor_left();
                            }
                            Action::CursorRight => {
                                state.cursor_right();
                            }
                            Action::CursorHome => {
                                state.cursor_home();
                            }
                            Action::CursorEnd => {
                                state.cursor_end();
                            }
                            Action::SendInput => {
                                let target = state.selected_agent().map(|a| a.target.clone());
                                if let Some(target) = target {
                                    let input = state.take_input();
                                    if !input.is_empty() {
                                        if let Err(e) = tmux_client.send_keys(&target, &input) {
                                            state.set_error(format!("Failed to send input: {}", e));
                                        } else if let Err(e) = tmux_client.send_keys(&target, "Enter") {
                                            state.set_error(format!("Failed to send Enter: {}", e));
                                        }
                                    }
                                }
                                // Stay in input mode for consecutive inputs
                            }
                            Action::SendNumber(num) => {
                                if let Some(agent) = state.selected_agent() {
                                    let target = agent.target.clone();
                                    let num_str = num.to_string();
                                    if let Err(e) = tmux_client.send_keys(&target, &num_str) {
                                        state.set_error(format!("Failed to send number: {}", e));
                                    } else if let Err(e) = tmux_client.send_keys(&target, "Enter") {
                                        state.set_error(format!("Failed to send Enter: {}", e));
                                    }
                                }
                            }
                            Action::SidebarWider => {
                                state.sidebar_width = (state.sidebar_width + 5).min(70);
                            }
                            Action::SidebarNarrower => {
                                state.sidebar_width = state.sidebar_width.saturating_sub(5).max(15);
                            }
                            Action::SelectAgent(idx) => {
                                state.select_agent(idx);
                            }
                            Action::ScrollUp => {
                                if state.show_help {
                                    state.help_scroll = state.help_scroll.saturating_sub(1);
                                } else if state.is_preview_focused() {
                                    state.preview_scroll = state.preview_scroll.saturating_sub(1);
                                } else {
                                    state.select_prev();
                                }
                            }
                            Action::ScrollDown => {
                                if state.show_help {
                                    state.help_scroll = state.help_scroll.saturating_add(1);
                                } else if state.is_preview_focused() {
                                    state.preview_scroll = state.preview_scroll.saturating_add(1);
                                } else {
                                    state.select_next();
                                }
                            }
                            Action::KillPanePending => {
                                // First 'd' press - set pending
                                if let Some(target) = state.selected_pane_target() {
                                    state.pending_kill = Some((target, std::time::Instant::now()));
                                }
                            }
                            Action::KillPane => {
                                // Second 'd' press confirmed - execute kill
                                if let Some((target, _)) = state.pending_kill.take() {
                                    if let Err(e) = tmux_client.kill_pane(&target) {
                                        state.set_error(format!("Failed to kill pane: {}", e));
                                    }
                                }
                            }
                            Action::SpawnStart => {
                                if let Some(session) = state.selected_session().map(|s| s.to_string()) {
                                    state.spawn_mode = Some(session);
                                }
                            }
                            Action::SpawnAgent(cmd) => {
                                if let Some(session) = state.spawn_mode.take() {
                                    // Look up cwd from any existing pane in the session
                                    let session_path = state
                                        .agents
                                        .root_agents
                                        .iter()
                                        .find(|a| a.session == session)
                                        .map(|a| a.path.clone())
                                        .or_else(|| {
                                            state.agents.non_agent_panes
                                                .iter()
                                                .find(|p| p.session == session)
                                                .map(|p| p.path.clone())
                                        });
                                    let cwd = session_path.as_deref();

                                    // Build window name: "{agent_type}-{basename_of_cwd}"
                                    let agent_short = cmd.split_whitespace().next().unwrap_or("agent");
                                    let dir_basename = cwd
                                        .and_then(|p| p.rsplit('/').find(|s| !s.is_empty()))
                                        .unwrap_or("agent");
                                    let window_name = format!("{}-{}", agent_short, dir_basename);

                                    if let Err(e) = tmux_client.new_window(&session, &window_name, &cmd, cwd) {
                                        state.set_error(format!("Failed to spawn: {}", e));
                                    }
                                }
                            }
                            Action::SpawnCancel => {
                                state.spawn_mode = None;
                            }
                            Action::RenameStart => {
                                if let Some(target) = state.selected_pane_target() {
                                    // Pre-fill input buffer with current window name
                                    let window_name = state
                                        .selected_pane_window_name()
                                        .unwrap_or_default();
                                    state.input_buffer = window_name.clone();
                                    state.cursor_position = window_name.len();
                                    state.rename_mode = Some(target);
                                }
                            }
                            Action::RenameExecute(name) => {
                                if let Some(target) = state.rename_mode.take() {
                                    let name = name.trim().to_string();
                                    if !name.is_empty() {
                                        if let Err(e) = tmux_client.rename_window(&target, &name) {
                                            state.set_error(format!("Failed to rename: {}", e));
                                        }
                                    }
                                    state.take_input();
                                }
                            }
                            Action::RenameCancel => {
                                state.rename_mode = None;
                                state.take_input();
                            }
                            Action::FlashFocusStart => {
                                state.flash_mode = Some(FlashMode::Focus);
                                state.flash_prefix = None;
                            }
                            Action::FlashGoStart => {
                                state.flash_mode = Some(FlashMode::Go);
                                state.flash_prefix = None;
                            }
                            Action::FlashCancel => {
                                state.flash_mode = None;
                                state.flash_prefix = None;
                            }
                            Action::FlashInput(c) => {
                                let targets = state.build_flash_targets();
                                let labels = generate_flash_labels(targets.len());
                                let is_go = matches!(state.flash_mode, Some(FlashMode::Go));

                                let resolved_idx = if let Some(prefix) = state.flash_prefix {
                                    // Two-char resolution
                                    let full_label = format!("{}{}", prefix, c);
                                    labels.iter().position(|l| *l == full_label)
                                } else {
                                    // Single-char check
                                    let c_str = c.to_string();
                                    if let Some(idx) = labels.iter().position(|l| *l == c_str) {
                                        Some(idx)
                                    } else if labels.iter().any(|l| l.starts_with(c)) {
                                        // Start of a two-char label
                                        state.flash_prefix = Some(c);
                                        None
                                    } else {
                                        // Invalid key, cancel
                                        state.flash_mode = None;
                                        state.flash_prefix = None;
                                        None
                                    }
                                };

                                if let Some(idx) = resolved_idx {
                                    if let Some(target) = targets.get(idx) {
                                        state.jump_to_flash_target(target);
                                        let should_focus_pane = is_go
                                            && !matches!(target, FlashTarget::InputArea);
                                        state.flash_mode = None;
                                        state.flash_prefix = None;
                                        if should_focus_pane {
                                            if let Some(pane_target) = state.selected_pane_target()
                                            {
                                                if let Err(e) =
                                                    tmux_client.focus_pane(&pane_target)
                                                {
                                                    state.set_error(format!(
                                                        "Failed to focus: {}",
                                                        e
                                                    ));
                                                } else if state.popup_mode {
                                                    tracing::info!("popup mode: quitting after flash-go");
                                                    state.should_quit = true;
                                                }
                                            }
                                        }
                                    } else {
                                        state.flash_mode = None;
                                        state.flash_prefix = None;
                                    }
                                } else if state.flash_prefix.is_none() {
                                    // Already cancelled above
                                }
                            }
                            Action::None => {}
                        }
                    }
                }
            }
        }

        if state.should_quit {
            tracing::info!("tmuxcc shutting down");
            break;
        }
    }

    Ok(())
}

pub(crate) fn map_key_to_action(
    code: KeyCode,
    modifiers: KeyModifiers,
    state: &AppState,
) -> Action {
    // If help is shown, handle scroll or close
    if state.show_help {
        return match code {
            KeyCode::Char('j') | KeyCode::Down => Action::ScrollDown,
            KeyCode::Char('k') | KeyCode::Up => Action::ScrollUp,
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('h') => {
                Action::HideHelp
            }
            _ => Action::None, // ignore other keys while help is open
        };
    }

    // If rename mode is active, handle rename keys
    if state.rename_mode.is_some() {
        return match code {
            KeyCode::Enter => {
                let name = state.get_input().to_string();
                Action::RenameExecute(name)
            }
            KeyCode::Esc => Action::RenameCancel,
            KeyCode::Backspace => Action::InputBackspace,
            KeyCode::Left => Action::CursorLeft,
            KeyCode::Right => Action::CursorRight,
            KeyCode::Home => Action::CursorHome,
            KeyCode::End => Action::CursorEnd,
            KeyCode::Char(c) => Action::InputChar(c),
            _ => Action::None,
        };
    }

    // If flash mode is active, handle flash keys
    if state.flash_mode.is_some() {
        return match code {
            KeyCode::Esc => Action::FlashCancel,
            KeyCode::Char(c) => Action::FlashInput(c),
            _ => Action::None,
        };
    }

    // If search mode is active, handle search keys
    if state.search_query.is_some() {
        return match code {
            KeyCode::Esc => Action::SearchCancel,
            KeyCode::Enter => Action::SearchConfirm,
            KeyCode::Backspace => Action::SearchBackspace,
            KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => Action::SearchNext,
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => Action::SearchPrev,
            KeyCode::Down => Action::SearchNext,
            KeyCode::Up => Action::SearchPrev,
            KeyCode::Tab => Action::FocusInput,
            KeyCode::BackTab => Action::FocusPreview,
            KeyCode::Right => Action::FocusInput,
            KeyCode::Char(c) => Action::SearchInput(c),
            _ => Action::None,
        };
    }

    // If spawn mode is active, handle spawn keys
    if state.spawn_mode.is_some() {
        return match code {
            KeyCode::Char('c') => {
                Action::SpawnAgent("claude --dangerously-skip-permissions".to_string())
            }
            KeyCode::Char('C') => Action::SpawnAgent("claude".to_string()),
            KeyCode::Char('x') => Action::SpawnAgent("codex".to_string()),
            KeyCode::Char('g') => Action::SpawnAgent("gemini".to_string()),
            KeyCode::Char('o') => Action::SpawnAgent("opencode".to_string()),
            KeyCode::Esc => Action::SpawnCancel,
            _ => Action::None,
        };
    }

    // If input panel is focused, handle input-specific keys
    if state.is_input_focused() {
        return match code {
            KeyCode::Esc => Action::FocusSidebar,
            KeyCode::Tab => Action::CycleFocus,
            KeyCode::BackTab => Action::FocusPreview,
            KeyCode::Enter if modifiers.contains(KeyModifiers::SHIFT) => Action::InputNewline,
            KeyCode::Enter if modifiers.contains(KeyModifiers::ALT) => Action::InputNewline,
            KeyCode::Enter => Action::SendInput,
            KeyCode::Backspace => Action::InputBackspace,
            KeyCode::Left => Action::CursorLeft,
            KeyCode::Right => Action::CursorRight,
            KeyCode::Home => Action::CursorHome,
            KeyCode::End => Action::CursorEnd,
            KeyCode::Char(c) => Action::InputChar(c),
            _ => Action::None,
        };
    }

    // If preview panel is focused, handle preview-specific keys
    if state.is_preview_focused() {
        return match code {
            KeyCode::Esc => Action::FocusSidebar,
            KeyCode::Tab => Action::CycleFocus,
            KeyCode::BackTab => Action::FocusSidebar,
            KeyCode::Char('j') | KeyCode::Down => Action::ScrollDown,
            KeyCode::Char('k') | KeyCode::Up => Action::ScrollUp,
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('?') | KeyCode::Char('h') => Action::ShowHelp,
            _ => Action::None,
        };
    }

    // Sidebar focused
    match code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,

        KeyCode::Char('j') | KeyCode::Down => Action::NextAgent,
        KeyCode::Char('k') | KeyCode::Up => Action::PrevAgent,
        KeyCode::Tab => Action::CycleFocus,
        KeyCode::BackTab => Action::FocusInput,

        // Left/Right arrows for focus navigation
        KeyCode::Right => Action::FocusInput,
        KeyCode::Left => Action::None, // Already on sidebar

        // Enter: collapse on session headers, focus pane on agents/panes
        KeyCode::Enter => {
            if matches!(state.cursor, TreeCursor::Session(_)) {
                Action::ToggleCollapse
            } else {
                Action::FocusPane
            }
        }

        // Multi-selection
        KeyCode::Char(' ') => Action::ToggleSelection,
        KeyCode::Char('a') if modifiers.contains(KeyModifiers::CONTROL) => Action::SelectAll,

        // Approval - y/Y approve, N (shift only) reject
        KeyCode::Char('y') | KeyCode::Char('Y') => Action::Approve,
        KeyCode::Char('N') => Action::Reject,
        KeyCode::Char('a') | KeyCode::Char('A') => Action::ApproveAll,

        // Number keys for quick choice selection (1-9)
        KeyCode::Char(c @ '1'..='9') => {
            let num = c.to_digit(10).unwrap() as u8;
            Action::SendNumber(num)
        }

        // Focus pane with 'f'
        KeyCode::Char('f') | KeyCode::Char('F') => Action::FocusPane,

        // Flash navigation
        KeyCode::Char('g') => Action::FlashFocusStart,
        KeyCode::Char('G') => Action::FlashGoStart,

        // Search
        KeyCode::Char('/') => Action::SearchStart,

        // Kill pane with 'dd' (double-tap)
        KeyCode::Char('d') => {
            if let Some((ref target, instant)) = state.pending_kill {
                if instant.elapsed().as_millis() < 400 {
                    // Check that we're still on the same target
                    if let Some(current_target) = state.selected_pane_target() {
                        if current_target == *target {
                            return Action::KillPane;
                        }
                    }
                }
            }
            Action::KillPanePending
        }

        // Spawn agent with 'n'
        KeyCode::Char('n') => Action::SpawnStart,

        // Collapse/expand all
        KeyCode::Char('[') => Action::CollapseAll,
        KeyCode::Char(']') => Action::ExpandAll,

        KeyCode::Char('M') => Action::ToggleNotifications,
        KeyCode::Char('H') => Action::ToggleHideNonAgentSessions,
        KeyCode::Char('V') => Action::ToggleHideNonAgentPanes,
        KeyCode::Char('s') => Action::CycleSortMode,
        KeyCode::Char('S') => Action::ToggleSubagentLog,
        KeyCode::Char('t') | KeyCode::Char('T') => Action::ToggleSummaryDetail,
        KeyCode::Char('p') => Action::TogglePrPanel,
        KeyCode::Char('o') => Action::OpenPrUrl,
        KeyCode::Char('c') => Action::CopyPrUrl,
        KeyCode::Char('r') => Action::Refresh,
        KeyCode::Char('R') => {
            // Rename when on a pane
            if state.selected_pane_target().is_some() {
                Action::RenameStart
            } else {
                Action::None
            }
        }

        // Sidebar resize (only < and >)
        KeyCode::Char('<') => Action::SidebarNarrower,
        KeyCode::Char('>') => Action::SidebarWider,

        KeyCode::Char('h') | KeyCode::Char('?') => Action::ShowHelp,

        KeyCode::Esc => {
            if !state.selected_agents.is_empty() {
                Action::ClearSelection
            } else if state.show_subagent_log {
                Action::ToggleSubagentLog
            } else {
                Action::Quit
            }
        }

        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn test_search_mode_tab_focuses_input() {
        let mut state = AppState::new();
        state.search_query = Some("test".to_string());

        let action = map_key_to_action(KeyCode::Tab, KeyModifiers::NONE, &state);
        assert_eq!(action, Action::FocusInput);
    }

    #[test]
    fn test_search_mode_right_focuses_input() {
        let mut state = AppState::new();
        state.search_query = Some("test".to_string());

        let action = map_key_to_action(KeyCode::Right, KeyModifiers::NONE, &state);
        assert_eq!(action, Action::FocusInput);
    }

    #[test]
    fn test_search_mode_backtab_focuses_preview() {
        let mut state = AppState::new();
        state.search_query = Some("test".to_string());

        let action = map_key_to_action(KeyCode::BackTab, KeyModifiers::SHIFT, &state);
        assert_eq!(action, Action::FocusPreview);
    }

    #[test]
    fn test_search_mode_esc_cancels() {
        let mut state = AppState::new();
        state.search_query = Some("test".to_string());

        let action = map_key_to_action(KeyCode::Esc, KeyModifiers::NONE, &state);
        assert_eq!(action, Action::SearchCancel);
    }

    #[test]
    fn test_search_mode_enter_confirms() {
        let mut state = AppState::new();
        state.search_query = Some("test".to_string());

        let action = map_key_to_action(KeyCode::Enter, KeyModifiers::NONE, &state);
        assert_eq!(action, Action::SearchConfirm);
    }

    #[test]
    fn test_search_mode_up_down_navigate() {
        let mut state = AppState::new();
        state.search_query = Some("test".to_string());

        assert_eq!(
            map_key_to_action(KeyCode::Down, KeyModifiers::NONE, &state),
            Action::SearchNext
        );
        assert_eq!(
            map_key_to_action(KeyCode::Up, KeyModifiers::NONE, &state),
            Action::SearchPrev
        );
    }

    #[test]
    fn test_sidebar_slash_starts_search() {
        let state = AppState::new();
        let action = map_key_to_action(KeyCode::Char('/'), KeyModifiers::NONE, &state);
        assert_eq!(action, Action::SearchStart);
    }

    #[test]
    fn test_sidebar_enter_on_agent_focuses() {
        let mut state = AppState::new();
        state.cursor = TreeCursor::Agent(0);
        let action = map_key_to_action(KeyCode::Enter, KeyModifiers::NONE, &state);
        assert_eq!(action, Action::FocusPane);
    }

    #[test]
    fn test_sidebar_enter_on_session_collapses() {
        let mut state = AppState::new();
        state.cursor = TreeCursor::Session("test".to_string());
        let action = map_key_to_action(KeyCode::Enter, KeyModifiers::NONE, &state);
        assert_eq!(action, Action::ToggleCollapse);
    }

    #[test]
    fn test_sidebar_esc_quits_when_no_selection() {
        let state = AppState::new();
        let action = map_key_to_action(KeyCode::Esc, KeyModifiers::NONE, &state);
        assert_eq!(action, Action::Quit);
    }

    #[test]
    fn test_sidebar_s_cycles_sort() {
        let state = AppState::new(); // sidebar focused by default
        let action = map_key_to_action(KeyCode::Char('s'), KeyModifiers::NONE, &state);
        assert_eq!(action, Action::CycleSortMode);
    }

    #[test]
    fn test_sidebar_shift_s_toggles_subagent_log() {
        let state = AppState::new();
        let action = map_key_to_action(KeyCode::Char('S'), KeyModifiers::SHIFT, &state);
        assert_eq!(action, Action::ToggleSubagentLog);
    }
}
