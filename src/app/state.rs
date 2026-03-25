use crate::agents::{AgentStatus, MonitoredAgent};
use crate::git::types::PrLookupResult;
use crate::monitor::SystemStats;
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// Sort mode for the sidebar tree view
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    /// Sort sessions by most recent activity (default)
    #[default]
    Activity,
    /// Sort sessions by agent status priority (Processing first, then AwaitingApproval, etc.)
    Status,
}

impl SortMode {
    /// Cycle to the next sort mode
    pub fn next(self) -> Self {
        match self {
            SortMode::Activity => SortMode::Status,
            SortMode::Status => SortMode::Activity,
        }
    }

    /// Return a human-readable label for the footer badge
    pub fn label(&self) -> &'static str {
        match self {
            SortMode::Activity => "Recent",
            SortMode::Status => "Status",
        }
    }
}

/// A pane that is not running a recognized agent
#[derive(Debug, Clone)]
pub struct NonAgentPane {
    pub target: String,
    pub session: String,
    pub window: u32,
    pub window_name: String,
    pub pane: u32,
    pub command: String,
    pub path: String,
}

/// Which panel is currently focused
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusedPanel {
    /// Agent list sidebar is focused
    #[default]
    Sidebar,
    /// Preview/message chain area is focused
    Preview,
    /// Input area is focused
    Input,
}

/// Semantic cursor that can point to either a session header, an agent, or a non-agent pane
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeCursor {
    /// Cursor is on a session header
    Session(String),
    /// Cursor is on an agent (index into root_agents)
    Agent(usize),
    /// Cursor is on a non-agent pane (index into non_agent_panes)
    NonAgentPane(usize),
}

impl Default for TreeCursor {
    fn default() -> Self {
        TreeCursor::Agent(0)
    }
}

/// Represents a navigable item in the tree
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavItem {
    Session(String),
    Agent(usize),
    NonAgentPane(usize),
}

/// Flash navigation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashMode {
    /// Flash-focus: jump cursor to target
    Focus,
    /// Flash-go: jump cursor + attach tmux
    Go,
}

/// Target for flash navigation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlashTarget {
    /// A navigable tree item
    Nav(NavItem),
    /// The input area
    InputArea,
}

/// Home-row priority keys for flash labels
const FLASH_KEYS: &[char] = &[
    'a', 's', 'd', 'f', 'j', 'k', 'l', 'h', 'e', 'w', 'r', 'u', 'i', 'o',
];

/// Prefix keys for two-char flash labels
const FLASH_PREFIXES: &[char] = &[';', ',', '.'];

/// Generate flash labels for a given number of targets
pub fn generate_flash_labels(count: usize) -> Vec<String> {
    let mut labels = Vec::with_capacity(count);
    // Single-char labels first
    for &c in FLASH_KEYS.iter().take(count.min(FLASH_KEYS.len())) {
        labels.push(c.to_string());
    }
    // Two-char labels with prefix keys for overflow
    if count > FLASH_KEYS.len() {
        'outer: for &prefix in FLASH_PREFIXES.iter() {
            for &c in FLASH_KEYS.iter() {
                if labels.len() >= count {
                    break 'outer;
                }
                labels.push(format!("{}{}", prefix, c));
            }
        }
    }
    labels
}

/// Tree structure containing all monitored agents
#[derive(Debug, Clone, Default)]
pub struct AgentTree {
    /// Root agents (directly in tmux panes)
    pub root_agents: Vec<MonitoredAgent>,
    /// Panes that don't match any agent parser
    pub non_agent_panes: Vec<NonAgentPane>,
}

impl AgentTree {
    /// Creates an empty agent tree
    pub fn new() -> Self {
        Self {
            root_agents: Vec::new(),
            non_agent_panes: Vec::new(),
        }
    }

    /// Returns the total number of agents (including subagents)
    pub fn total_count(&self) -> usize {
        self.root_agents.iter().map(|a| 1 + a.subagents.len()).sum()
    }

    /// Returns the number of active agents (those needing attention)
    pub fn active_count(&self) -> usize {
        self.root_agents
            .iter()
            .filter(|a| a.status.needs_attention())
            .count()
    }

    /// Returns the total number of running subagents
    pub fn running_subagent_count(&self) -> usize {
        use crate::agents::SubagentStatus;
        self.root_agents
            .iter()
            .flat_map(|a| &a.subagents)
            .filter(|s| matches!(s.status, SubagentStatus::Running))
            .count()
    }

    /// Returns the number of processing agents
    pub fn processing_count(&self) -> usize {
        use crate::agents::AgentStatus;
        self.root_agents
            .iter()
            .filter(|a| matches!(a.status, AgentStatus::Processing { .. }))
            .count()
    }

    /// Gets an agent by index (for selection)
    pub fn get_agent(&self, index: usize) -> Option<&MonitoredAgent> {
        self.root_agents.get(index)
    }

    /// Gets a mutable agent by index
    pub fn get_agent_mut(&mut self, index: usize) -> Option<&mut MonitoredAgent> {
        self.root_agents.get_mut(index)
    }
}

/// Spinner frames for animation
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Main application state
#[derive(Debug)]
pub struct AppState {
    /// Tree of monitored agents
    pub agents: AgentTree,
    /// All tmux session names (including those without agents)
    pub all_sessions: Vec<String>,
    /// Semantic cursor (session header, agent, or non-agent pane)
    pub cursor: TreeCursor,
    /// Multi-selected agent indices
    pub selected_agents: HashSet<usize>,
    /// Collapsed session names
    pub collapsed_sessions: HashSet<String>,
    /// Which panel is focused
    pub focused_panel: FocusedPanel,
    /// Input buffer (always available)
    pub input_buffer: String,
    /// Cursor position within input buffer (byte offset)
    pub cursor_position: usize,
    /// Whether help is being shown
    pub show_help: bool,
    /// Help scroll offset
    pub help_scroll: u16,
    /// Whether subagent log is shown
    pub show_subagent_log: bool,
    /// Whether summary detail (TODOs and Tools) is shown
    pub show_summary_detail: bool,
    /// Whether the application should quit
    pub should_quit: bool,
    /// Last error message (if any)
    pub last_error: Option<String>,
    /// Sidebar width in percentage (15-70)
    pub sidebar_width: u16,
    /// Animation tick counter
    pub tick: usize,
    /// Last tick time for animation throttling
    last_tick: Instant,
    /// System resource statistics
    pub system_stats: SystemStats,
    /// Pending kill confirmation: (target, timestamp)
    pub pending_kill: Option<(String, Instant)>,
    /// Spawn mode: Some(session_name) when active
    pub spawn_mode: Option<String>,
    /// Whether to hide sessions that have no agents
    pub hide_non_agent_sessions: bool,
    /// Whether to hide non-agent panes within sessions
    pub hide_non_agent_panes: bool,
    /// Running inside a tmux popup (auto-quit on focus/go)
    pub popup_mode: bool,
    /// Rename mode: Some(target) when active
    pub rename_mode: Option<String>,
    /// Preview scroll offset
    pub preview_scroll: u16,
    /// Search mode: active search query
    pub search_query: Option<String>,
    /// Cursor position before search started (for restore on cancel)
    pub pre_search_cursor: Option<TreeCursor>,
    /// Flash navigation mode
    pub flash_mode: Option<FlashMode>,
    /// First character of a two-char flash label (waiting for second char)
    pub flash_prefix: Option<char>,
    /// PR info per agent working directory path
    pub pr_info: HashMap<String, PrLookupResult>,
    /// Whether PR detail panel is shown
    pub show_pr_panel: bool,
    /// Paths for which the PR panel has been auto-opened
    pr_auto_opened: HashSet<String>,
    /// Whether desktop notifications are enabled (mirrors Notifier state for UI display)
    pub notifications_enabled: bool,
    /// Active notification sound profile name (mirrors config for UI display)
    pub notification_profile: String,
    /// Current sort mode for the sidebar tree
    pub sort_mode: SortMode,
}

impl AppState {
    /// Creates a new AppState with default settings
    pub fn new() -> Self {
        Self {
            agents: AgentTree::new(),
            all_sessions: Vec::new(),
            cursor: TreeCursor::default(),
            selected_agents: HashSet::new(),
            collapsed_sessions: HashSet::new(),
            focused_panel: FocusedPanel::Sidebar,
            input_buffer: String::new(),
            cursor_position: 0,
            show_help: false,
            help_scroll: 0,
            show_subagent_log: false,
            show_summary_detail: false,
            should_quit: false,
            last_error: None,
            sidebar_width: 35,
            tick: 0,
            last_tick: Instant::now(),
            system_stats: SystemStats::new(),
            pending_kill: None,
            spawn_mode: None,
            rename_mode: None,
            hide_non_agent_sessions: true,
            hide_non_agent_panes: true,
            popup_mode: false,
            preview_scroll: 0,
            search_query: None,
            pre_search_cursor: None,
            flash_mode: None,
            flash_prefix: None,
            pr_info: HashMap::new(),
            show_pr_panel: false,
            pr_auto_opened: HashSet::new(),
            notifications_enabled: true,
            notification_profile: "default".to_string(),
            sort_mode: SortMode::default(),
        }
    }

    /// Build the flat navigation list: session headers + visible agents + non-agent panes in display order.
    /// Uses IndexMap to preserve the insertion order from the sorted agents list.
    pub fn build_nav_items(&self) -> Vec<NavItem> {
        // Group agents by session, preserving insertion order (from sorted agents)
        let mut agent_sessions: IndexMap<&str, Vec<usize>> = IndexMap::new();
        for (idx, agent) in self.agents.root_agents.iter().enumerate() {
            agent_sessions.entry(&agent.session).or_default().push(idx);
        }

        // Group non-agent panes by session, preserving insertion order
        let mut nap_sessions: IndexMap<&str, Vec<usize>> = IndexMap::new();
        for (idx, nap) in self.agents.non_agent_panes.iter().enumerate() {
            nap_sessions.entry(&nap.session).or_default().push(idx);
        }

        // Collect all session names preserving the order from agents first,
        // then non-agent panes, then all_sessions for any remaining
        let mut all_session_names: IndexMap<&str, ()> = IndexMap::new();
        for s in agent_sessions.keys() {
            all_session_names.insert(s, ());
        }
        for s in nap_sessions.keys() {
            all_session_names.insert(s, ());
        }
        for s in &self.all_sessions {
            all_session_names.insert(s.as_str(), ());
        }

        // Check if search is active with a non-empty query
        let search_filter = self
            .search_query
            .as_ref()
            .filter(|q| !q.is_empty())
            .map(|q| q.to_lowercase());

        let mut items = Vec::new();
        for session in all_session_names.keys() {
            // Skip sessions with no agents when hide_non_agent_sessions is enabled
            if self.hide_non_agent_sessions && !agent_sessions.contains_key(session) {
                continue;
            }

            // Collect child items for this session
            let mut session_children = Vec::new();
            if !self.collapsed_sessions.contains(*session) {
                if let Some(agent_indices) = agent_sessions.get(session) {
                    for &idx in agent_indices {
                        session_children.push(NavItem::Agent(idx));
                    }
                }
                if !self.hide_non_agent_panes {
                    if let Some(nap_indices) = nap_sessions.get(session) {
                        for &idx in nap_indices {
                            session_children.push(NavItem::NonAgentPane(idx));
                        }
                    }
                }
            }

            if let Some(ref query) = search_filter {
                // Filter mode: only show sessions with matching children or matching name
                let session_matches = session.to_lowercase().contains(query);
                let matching_children: Vec<NavItem> = session_children
                    .into_iter()
                    .filter(|item| {
                        let text = self.nav_item_text(item).to_lowercase();
                        text.contains(query)
                    })
                    .collect();

                if session_matches || !matching_children.is_empty() {
                    items.push(NavItem::Session(session.to_string()));
                    if session_matches && matching_children.is_empty() {
                        // Session name matches but no children match — show all children
                        // (re-collect since we consumed them)
                        if !self.collapsed_sessions.contains(*session) {
                            if let Some(agent_indices) = agent_sessions.get(session) {
                                for &idx in agent_indices {
                                    items.push(NavItem::Agent(idx));
                                }
                            }
                            if !self.hide_non_agent_panes {
                                if let Some(nap_indices) = nap_sessions.get(session) {
                                    for &idx in nap_indices {
                                        items.push(NavItem::NonAgentPane(idx));
                                    }
                                }
                            }
                        }
                    } else {
                        items.extend(matching_children);
                    }
                }
            } else {
                // Normal mode: show everything
                items.push(NavItem::Session(session.to_string()));
                items.extend(session_children);
            }
        }
        items
    }

    /// Returns the agent index when cursor is on an agent
    pub fn selected_agent_index(&self) -> Option<usize> {
        match &self.cursor {
            TreeCursor::Agent(idx) => Some(*idx),
            _ => None,
        }
    }

    /// Returns the non-agent pane index when cursor is on one
    pub fn selected_non_agent_index(&self) -> Option<usize> {
        match &self.cursor {
            TreeCursor::NonAgentPane(idx) => Some(*idx),
            _ => None,
        }
    }

    /// Returns the currently selected non-agent pane
    pub fn selected_non_agent_pane(&self) -> Option<&NonAgentPane> {
        self.selected_non_agent_index()
            .and_then(|idx| self.agents.non_agent_panes.get(idx))
    }

    /// Returns the currently selected agent (None when cursor is on a session or non-agent pane)
    pub fn selected_agent(&self) -> Option<&MonitoredAgent> {
        self.selected_agent_index()
            .and_then(|idx| self.agents.get_agent(idx))
    }

    /// Returns the currently selected agent mutably
    pub fn selected_agent_mut(&mut self) -> Option<&mut MonitoredAgent> {
        self.selected_agent_index()
            .and_then(|idx| self.agents.get_agent_mut(idx))
    }

    /// Returns the target of whatever pane the cursor is on (agent or non-agent)
    pub fn selected_pane_target(&self) -> Option<String> {
        match &self.cursor {
            TreeCursor::Agent(idx) => self.agents.get_agent(*idx).map(|a| a.target.clone()),
            TreeCursor::NonAgentPane(idx) => self
                .agents
                .non_agent_panes
                .get(*idx)
                .map(|p| p.target.clone()),
            TreeCursor::Session(_) => None,
        }
    }

    /// Returns the window name of whatever pane the cursor is on
    pub fn selected_pane_window_name(&self) -> Option<String> {
        match &self.cursor {
            TreeCursor::Agent(idx) => self.agents.get_agent(*idx).map(|a| a.window_name.clone()),
            TreeCursor::NonAgentPane(idx) => self
                .agents
                .non_agent_panes
                .get(*idx)
                .map(|p| p.window_name.clone()),
            TreeCursor::Session(_) => None,
        }
    }

    /// Returns the session name the cursor is on or in
    pub fn selected_session(&self) -> Option<&str> {
        match &self.cursor {
            TreeCursor::Session(s) => Some(s),
            TreeCursor::Agent(idx) => self.agents.get_agent(*idx).map(|a| a.session.as_str()),
            TreeCursor::NonAgentPane(idx) => self
                .agents
                .non_agent_panes
                .get(*idx)
                .map(|p| p.session.as_str()),
        }
    }

    /// Advance the animation tick (throttled to ~10fps for spinner)
    pub fn tick(&mut self) {
        const TICK_INTERVAL_MS: u128 = 80; // ~12fps for smooth spinner
        if self.last_tick.elapsed().as_millis() >= TICK_INTERVAL_MS {
            self.tick = self.tick.wrapping_add(1);
            self.last_tick = Instant::now();
        }
    }

    /// Get the current spinner frame
    pub fn spinner_frame(&self) -> &'static str {
        SPINNER_FRAMES[self.tick % SPINNER_FRAMES.len()]
    }

    /// Check if input panel is focused
    pub fn is_input_focused(&self) -> bool {
        self.focused_panel == FocusedPanel::Input
    }

    /// Check if preview panel is focused
    pub fn is_preview_focused(&self) -> bool {
        self.focused_panel == FocusedPanel::Preview
    }

    /// Check if sidebar panel is focused
    pub fn is_sidebar_focused(&self) -> bool {
        self.focused_panel == FocusedPanel::Sidebar
    }

    /// Focus on the input panel
    pub fn focus_input(&mut self) {
        self.focused_panel = FocusedPanel::Input;
    }

    /// Focus on the sidebar
    pub fn focus_sidebar(&mut self) {
        self.focused_panel = FocusedPanel::Sidebar;
    }

    /// Focus on the preview
    pub fn focus_preview(&mut self) {
        self.focused_panel = FocusedPanel::Preview;
    }

    /// Cycle focus: Sidebar → Preview → Input → Sidebar
    pub fn toggle_focus(&mut self) {
        self.focused_panel = match self.focused_panel {
            FocusedPanel::Sidebar => FocusedPanel::Preview,
            FocusedPanel::Preview => FocusedPanel::Input,
            FocusedPanel::Input => FocusedPanel::Sidebar,
        };
    }

    /// Add a character to the input buffer at cursor position
    pub fn input_char(&mut self, c: char) {
        self.input_buffer.insert(self.cursor_position, c);
        self.cursor_position += c.len_utf8();
    }

    /// Add a newline to the input buffer at cursor position
    pub fn input_newline(&mut self) {
        self.input_buffer.insert(self.cursor_position, '\n');
        self.cursor_position += 1;
    }

    /// Delete the character before the cursor
    pub fn input_backspace(&mut self) {
        if self.cursor_position > 0 {
            // Find the previous character boundary
            let prev_boundary = self.input_buffer[..self.cursor_position]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input_buffer.remove(prev_boundary);
            self.cursor_position = prev_boundary;
        }
    }

    /// Get the current input buffer
    pub fn get_input(&self) -> &str {
        &self.input_buffer
    }

    /// Get the current cursor position
    pub fn get_cursor_position(&self) -> usize {
        self.cursor_position
    }

    /// Take and clear the input buffer
    pub fn take_input(&mut self) -> String {
        self.cursor_position = 0;
        std::mem::take(&mut self.input_buffer)
    }

    /// Move cursor left by one character
    pub fn cursor_left(&mut self) {
        if self.cursor_position > 0 {
            // Find the previous character boundary
            self.cursor_position = self.input_buffer[..self.cursor_position]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    /// Move cursor right by one character
    pub fn cursor_right(&mut self) {
        if self.cursor_position < self.input_buffer.len() {
            // Find the next character boundary
            if let Some(c) = self.input_buffer[self.cursor_position..].chars().next() {
                self.cursor_position += c.len_utf8();
            }
        }
    }

    /// Move cursor to the beginning of the input
    pub fn cursor_home(&mut self) {
        self.cursor_position = 0;
    }

    /// Move cursor to the end of the input
    pub fn cursor_end(&mut self) {
        self.cursor_position = self.input_buffer.len();
    }

    /// Selects the next item in navigation order
    pub fn select_next(&mut self) {
        let nav_items = self.build_nav_items();
        if nav_items.is_empty() {
            return;
        }

        // Find current position
        let current_pos = self.find_nav_position(&nav_items);
        let next_pos = match current_pos {
            Some(pos) => (pos + 1) % nav_items.len(),
            None => 0,
        };

        self.set_cursor_from_nav(&nav_items[next_pos]);
    }

    /// Selects the previous item in navigation order
    pub fn select_prev(&mut self) {
        let nav_items = self.build_nav_items();
        if nav_items.is_empty() {
            return;
        }

        let current_pos = self.find_nav_position(&nav_items);
        let prev_pos = match current_pos {
            Some(pos) => {
                if pos == 0 {
                    nav_items.len() - 1
                } else {
                    pos - 1
                }
            }
            None => 0,
        };

        self.set_cursor_from_nav(&nav_items[prev_pos]);
    }

    /// Find the current cursor's position in the nav items list
    fn find_nav_position(&self, nav_items: &[NavItem]) -> Option<usize> {
        nav_items
            .iter()
            .position(|item| match (&self.cursor, item) {
                (TreeCursor::Session(s1), NavItem::Session(s2)) => s1 == s2,
                (TreeCursor::Agent(i1), NavItem::Agent(i2)) => i1 == i2,
                (TreeCursor::NonAgentPane(i1), NavItem::NonAgentPane(i2)) => i1 == i2,
                _ => false,
            })
    }

    /// Set cursor from a NavItem
    pub fn set_cursor_from_nav(&mut self, item: &NavItem) {
        self.cursor = match item {
            NavItem::Session(s) => TreeCursor::Session(s.clone()),
            NavItem::Agent(idx) => TreeCursor::Agent(*idx),
            NavItem::NonAgentPane(idx) => TreeCursor::NonAgentPane(*idx),
        };
        self.preview_scroll = 0;
    }

    /// Build the list of flash targets (nav items + input area)
    pub fn build_flash_targets(&self) -> Vec<FlashTarget> {
        let mut targets: Vec<FlashTarget> = self
            .build_nav_items()
            .into_iter()
            .map(FlashTarget::Nav)
            .collect();
        targets.push(FlashTarget::InputArea);
        targets
    }

    /// Get searchable text for a nav item (session name, agent path, pane command)
    pub fn nav_item_text(&self, item: &NavItem) -> String {
        match item {
            NavItem::Session(s) => s.clone(),
            NavItem::Agent(idx) => self
                .agents
                .get_agent(*idx)
                .map(|a| format!("{} {} {}", a.session, a.window_name, a.abbreviated_path()))
                .unwrap_or_default(),
            NavItem::NonAgentPane(idx) => self
                .agents
                .non_agent_panes
                .get(*idx)
                .map(|p| format!("{} {} {}", p.session, p.window_name, p.command))
                .unwrap_or_default(),
        }
    }

    /// Get nav items filtered by search query. Jumps cursor to first match.
    pub fn apply_search(&mut self) {
        if let Some(ref query) = self.search_query {
            if query.is_empty() {
                return;
            }
            let lower_query = query.to_lowercase();
            let nav_items = self.build_nav_items();
            // Find first matching item and jump to it
            for item in &nav_items {
                let text = self.nav_item_text(item).to_lowercase();
                if text.contains(&lower_query) {
                    self.set_cursor_from_nav(item);
                    return;
                }
            }
        }
    }

    /// Jump cursor to the next search match after the current position
    pub fn search_next(&mut self) {
        if let Some(ref query) = self.search_query {
            if query.is_empty() {
                return;
            }
            let lower_query = query.to_lowercase();
            let nav_items = self.build_nav_items();
            let current_pos = self.find_nav_position(&nav_items).unwrap_or(0);

            // Search from current+1, wrapping around
            for i in 1..=nav_items.len() {
                let idx = (current_pos + i) % nav_items.len();
                let text = self.nav_item_text(&nav_items[idx]).to_lowercase();
                if text.contains(&lower_query) {
                    self.set_cursor_from_nav(&nav_items[idx]);
                    return;
                }
            }
        }
    }

    /// Jump cursor to the previous search match before the current position
    pub fn search_prev(&mut self) {
        if let Some(ref query) = self.search_query {
            if query.is_empty() {
                return;
            }
            let lower_query = query.to_lowercase();
            let nav_items = self.build_nav_items();
            let current_pos = self.find_nav_position(&nav_items).unwrap_or(0);

            // Search backwards from current-1, wrapping around
            for i in 1..=nav_items.len() {
                let idx = (current_pos + nav_items.len() - i) % nav_items.len();
                let text = self.nav_item_text(&nav_items[idx]).to_lowercase();
                if text.contains(&lower_query) {
                    self.set_cursor_from_nav(&nav_items[idx]);
                    return;
                }
            }
        }
    }

    /// Check if a nav item matches the current search query
    pub fn matches_search(&self, item: &NavItem) -> bool {
        if let Some(ref query) = self.search_query {
            if query.is_empty() {
                return false;
            }
            let text = self.nav_item_text(item).to_lowercase();
            text.contains(&query.to_lowercase())
        } else {
            false
        }
    }

    /// Jump cursor to a flash target
    pub fn jump_to_flash_target(&mut self, target: &FlashTarget) {
        match target {
            FlashTarget::Nav(item) => self.set_cursor_from_nav(item),
            FlashTarget::InputArea => self.focus_input(),
        }
    }

    /// Selects an agent by index
    pub fn select_agent(&mut self, index: usize) {
        if index < self.agents.root_agents.len() {
            self.cursor = TreeCursor::Agent(index);
            self.preview_scroll = 0;
        }
    }

    /// Toggles selection of the current agent
    pub fn toggle_selection(&mut self) {
        if let Some(idx) = self.selected_agent_index() {
            if self.selected_agents.contains(&idx) {
                self.selected_agents.remove(&idx);
            } else {
                self.selected_agents.insert(idx);
            }
        }
    }

    /// Selects all agents
    pub fn select_all(&mut self) {
        for i in 0..self.agents.root_agents.len() {
            self.selected_agents.insert(i);
        }
    }

    /// Clears all selections
    pub fn clear_selection(&mut self) {
        self.selected_agents.clear();
    }

    /// Returns indices to operate on (selected agents, or current if none selected)
    pub fn get_operation_indices(&self) -> Vec<usize> {
        if self.selected_agents.is_empty() {
            if let Some(idx) = self.selected_agent_index() {
                vec![idx]
            } else {
                vec![]
            }
        } else {
            let mut indices: Vec<usize> = self.selected_agents.iter().copied().collect();
            indices.sort();
            indices
        }
    }

    /// Check if an agent is in multi-selection
    pub fn is_multi_selected(&self, index: usize) -> bool {
        self.selected_agents.contains(&index)
    }

    /// Toggle collapse of the current session
    pub fn toggle_collapse(&mut self) {
        if let Some(session) = self.selected_session().map(|s| s.to_string()) {
            if self.collapsed_sessions.contains(&session) {
                self.collapsed_sessions.remove(&session);
            } else {
                self.collapsed_sessions.insert(session);
            }
        }
    }

    /// Collapse all sessions
    pub fn collapse_all(&mut self) {
        for session in &self.all_sessions {
            self.collapsed_sessions.insert(session.clone());
        }
    }

    /// Expand all sessions
    pub fn expand_all(&mut self) {
        self.collapsed_sessions.clear();
    }

    /// Toggles help display
    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
        if self.show_help {
            self.help_scroll = 0;
        }
    }

    /// Toggles subagent log display
    pub fn toggle_subagent_log(&mut self) {
        self.show_subagent_log = !self.show_subagent_log;
    }

    /// Toggles summary detail (TODOs and Tools) display
    pub fn toggle_summary_detail(&mut self) {
        self.show_summary_detail = !self.show_summary_detail;
    }

    /// Toggles PR detail panel display
    pub fn toggle_pr_panel(&mut self) {
        self.show_pr_panel = !self.show_pr_panel;
    }

    /// Get PR lookup result for the currently selected agent
    pub fn selected_agent_pr(&self) -> Option<&PrLookupResult> {
        self.selected_agent()
            .and_then(|a| self.pr_info.get(&a.path))
    }

    /// Get PrInfo if the selected agent has an open PR
    pub fn selected_pr(&self) -> Option<&crate::git::types::PrInfo> {
        match self.selected_agent_pr() {
            Some(PrLookupResult::Found(info)) => Some(info),
            _ => None,
        }
    }

    /// Handle auto-open logic when PR data arrives
    pub fn handle_pr_auto_open(&mut self) {
        if let Some(agent) = self.selected_agent() {
            let path = agent.path.clone();
            if let Some(PrLookupResult::Found(_)) = self.pr_info.get(&path) {
                if !self.pr_auto_opened.contains(&path) {
                    self.show_pr_panel = true;
                    self.pr_auto_opened.insert(path);
                }
            }
        }
    }

    /// Sets an error message
    pub fn set_error(&mut self, message: String) {
        self.last_error = Some(message);
    }

    /// Clears the error message
    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    /// Clamp cursor to valid range after agents update
    pub fn clamp_cursor(&mut self) {
        match &self.cursor {
            TreeCursor::Agent(idx) => {
                if *idx >= self.agents.root_agents.len() {
                    if self.agents.root_agents.is_empty() {
                        self.cursor = TreeCursor::Agent(0);
                    } else {
                        self.cursor = TreeCursor::Agent(self.agents.root_agents.len() - 1);
                    }
                }
            }
            TreeCursor::NonAgentPane(idx) => {
                if *idx >= self.agents.non_agent_panes.len() {
                    if self.agents.non_agent_panes.is_empty() {
                        self.cursor = TreeCursor::Agent(0);
                    } else {
                        self.cursor =
                            TreeCursor::NonAgentPane(self.agents.non_agent_panes.len() - 1);
                    }
                }
            }
            TreeCursor::Session(session) => {
                // Check if session still exists in agents, non-agent panes, or all_sessions
                let exists = self
                    .agents
                    .root_agents
                    .iter()
                    .any(|a| a.session == *session)
                    || self
                        .agents
                        .non_agent_panes
                        .iter()
                        .any(|p| p.session == *session)
                    || self.all_sessions.iter().any(|s| s == session);
                if !exists {
                    self.cursor = TreeCursor::Agent(0);
                }
            }
        }
    }

    /// Cycle to the next sort mode, preserving the currently selected agent
    pub fn cycle_sort_mode(&mut self) {
        // Save the current agent's target before sorting
        let saved_target = self.selected_pane_target();

        self.sort_mode = self.sort_mode.next();
        sort_agents(&mut self.agents.root_agents, self.sort_mode);

        // Restore cursor to the same agent after re-sorting
        if let Some(target) = saved_target {
            if let Some(new_idx) = self
                .agents
                .root_agents
                .iter()
                .position(|a| a.target == target)
            {
                self.cursor = TreeCursor::Agent(new_idx);
            }
        }
    }
}

/// Sort agents according to the given sort mode
pub fn sort_agents(agents: &mut Vec<MonitoredAgent>, mode: SortMode) {
    // Group agents by session, preserving within-session order
    let mut session_agents: IndexMap<String, Vec<MonitoredAgent>> = IndexMap::new();
    for agent in agents.drain(..) {
        session_agents
            .entry(agent.session.clone())
            .or_default()
            .push(agent);
    }

    // Sort agents within each session by target (window/pane order)
    for group in session_agents.values_mut() {
        group.sort_by(|a, b| a.target.cmp(&b.target));
    }

    // Compute session ordering key and sort sessions
    let mut session_order: Vec<(String, u8, Instant)> = session_agents
        .iter()
        .map(|(session, group)| {
            let most_recent = group
                .iter()
                .map(|a| a.last_updated)
                .max()
                .unwrap_or_else(Instant::now);
            let priority = match mode {
                SortMode::Activity => 0, // all same priority; sort only by recency
                SortMode::Status => session_status_priority(group),
            };
            (session.clone(), priority, most_recent)
        })
        .collect();

    // Sort by priority first, then by most recent activity (most recent first)
    session_order.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| b.2.cmp(&a.2)));

    // Rebuild agents in sorted order
    for (session, _, _) in session_order {
        if let Some(group) = session_agents.swap_remove(&session) {
            agents.extend(group);
        }
    }
}

/// Compute status priority for a session's agents (lower = higher priority)
fn session_status_priority(agents: &[MonitoredAgent]) -> u8 {
    agents
        .iter()
        .map(|a| agent_status_priority(&a.status))
        .min()
        .unwrap_or(4)
}

/// Priority for a single agent status (lower = higher priority)
fn agent_status_priority(status: &AgentStatus) -> u8 {
    match status {
        AgentStatus::Processing { .. } => 0,
        AgentStatus::AwaitingApproval { .. } => 1,
        AgentStatus::Error { .. } => 2,
        AgentStatus::Unknown => 3,
        AgentStatus::Idle => 4,
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentType;

    fn make_agent(session: &str, target: &str, window: u32, pane: u32) -> MonitoredAgent {
        MonitoredAgent::new(
            format!("{}-1", target),
            target.to_string(),
            session.to_string(),
            window,
            "code".to_string(),
            pane,
            "/home/user/project".to_string(),
            AgentType::ClaudeCode,
            1000,
        )
    }

    #[test]
    fn test_app_state_navigation() {
        let mut state = AppState::new();

        // Add some agents in same session
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.0", 0, 0));
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.1", 0, 1));

        // Cursor starts at Agent(0)
        assert_eq!(state.cursor, TreeCursor::Agent(0));

        // Next: Agent(0) -> Agent(1)
        state.select_next();
        assert_eq!(state.selected_agent_index(), Some(1));

        // Next wraps: Agent(1) -> Session("main")
        state.select_next();
        assert_eq!(state.cursor, TreeCursor::Session("main".to_string()));

        // Next: Session("main") -> Agent(0)
        state.select_next();
        assert_eq!(state.selected_agent_index(), Some(0));

        // Prev wraps: Agent(0) -> Session("main")
        state.select_prev();
        assert_eq!(state.cursor, TreeCursor::Session("main".to_string()));

        // Prev: Session("main") -> Agent(1) (wrap to end)
        state.select_prev();
        assert_eq!(state.selected_agent_index(), Some(1));
    }

    #[test]
    fn test_selected_session() {
        let mut state = AppState::new();
        state
            .agents
            .root_agents
            .push(make_agent("dev", "dev:0.0", 0, 0));

        state.cursor = TreeCursor::Session("dev".to_string());
        assert_eq!(state.selected_session(), Some("dev"));
        assert_eq!(state.selected_agent_index(), None);
        assert!(state.selected_agent().is_none());

        state.cursor = TreeCursor::Agent(0);
        assert_eq!(state.selected_session(), Some("dev"));
        assert_eq!(state.selected_agent_index(), Some(0));
        assert!(state.selected_agent().is_some());
    }

    #[test]
    fn test_toggle_pr_panel() {
        let mut state = AppState::new();
        assert!(!state.show_pr_panel);
        state.toggle_pr_panel();
        assert!(state.show_pr_panel);
        state.toggle_pr_panel();
        assert!(!state.show_pr_panel);
    }

    #[test]
    fn test_selected_agent_pr_found() {
        let mut state = AppState::new();
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.0", 0, 0));
        state.cursor = TreeCursor::Agent(0);

        let pr = crate::git::types::PrInfo {
            number: 42,
            title: "Test PR".to_string(),
            state: "OPEN".to_string(),
            url: "https://github.com/test/42".to_string(),
            head_ref: "feature".to_string(),
            base_ref: "main".to_string(),
            is_draft: false,
            review_decision: crate::git::types::ReviewDecision::Approved,
            mergeable: crate::git::types::MergeableState::Mergeable,
            checks: vec![],
            total_comments: 0,
            additions: 10,
            deletions: 5,
        };
        state
            .pr_info
            .insert("/home/user/project".to_string(), PrLookupResult::Found(pr));

        assert!(state.selected_pr().is_some());
        assert_eq!(state.selected_pr().unwrap().number, 42);
    }

    #[test]
    fn test_selected_agent_pr_none() {
        let mut state = AppState::new();
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.0", 0, 0));
        state.cursor = TreeCursor::Agent(0);
        assert!(state.selected_pr().is_none());
    }

    #[test]
    fn test_pr_auto_open() {
        let mut state = AppState::new();
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.0", 0, 0));
        state.cursor = TreeCursor::Agent(0);

        let pr = crate::git::types::PrInfo {
            number: 1,
            title: "t".to_string(),
            state: "OPEN".to_string(),
            url: "u".to_string(),
            head_ref: "h".to_string(),
            base_ref: "b".to_string(),
            is_draft: false,
            review_decision: crate::git::types::ReviewDecision::Unknown,
            mergeable: crate::git::types::MergeableState::Unknown,
            checks: vec![],
            total_comments: 0,
            additions: 0,
            deletions: 0,
        };

        // First detection auto-opens
        state.pr_info.insert(
            "/home/user/project".to_string(),
            PrLookupResult::Found(pr.clone()),
        );
        state.handle_pr_auto_open();
        assert!(state.show_pr_panel);

        // User closes, second detection does NOT reopen
        state.show_pr_panel = false;
        state.handle_pr_auto_open();
        assert!(!state.show_pr_panel);
    }

    #[test]
    fn test_multiple_agents_same_path() {
        let mut state = AppState::new();
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.0", 0, 0));
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.1", 0, 1));

        let pr = crate::git::types::PrInfo {
            number: 99,
            title: "shared".to_string(),
            state: "OPEN".to_string(),
            url: "u".to_string(),
            head_ref: "h".to_string(),
            base_ref: "b".to_string(),
            is_draft: false,
            review_decision: crate::git::types::ReviewDecision::Unknown,
            mergeable: crate::git::types::MergeableState::Unknown,
            checks: vec![],
            total_comments: 0,
            additions: 0,
            deletions: 0,
        };
        state
            .pr_info
            .insert("/home/user/project".to_string(), PrLookupResult::Found(pr));

        // Both agents at same path see the PR
        state.cursor = TreeCursor::Agent(0);
        assert_eq!(state.selected_pr().unwrap().number, 99);
        state.cursor = TreeCursor::Agent(1);
        assert_eq!(state.selected_pr().unwrap().number, 99);
    }

    #[test]
    fn test_collapsed_navigation() {
        let mut state = AppState::new();
        state
            .agents
            .root_agents
            .push(make_agent("alpha", "alpha:0.0", 0, 0));
        state
            .agents
            .root_agents
            .push(make_agent("beta", "beta:0.0", 0, 0));

        // Collapse alpha session
        state.collapsed_sessions.insert("alpha".to_string());

        // Nav items should be: Session(alpha), Session(beta), Agent(1)
        let nav = state.build_nav_items();
        assert_eq!(nav.len(), 3);
        assert_eq!(nav[0], NavItem::Session("alpha".to_string()));
        assert_eq!(nav[1], NavItem::Session("beta".to_string()));
        assert_eq!(nav[2], NavItem::Agent(1));
    }

    #[test]
    fn test_hide_non_agent_panes() {
        let mut state = AppState::new();
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.0", 0, 0));
        state.agents.non_agent_panes.push(NonAgentPane {
            target: "main:1.0".to_string(),
            session: "main".to_string(),
            window: 1,
            window_name: "nvim".to_string(),
            pane: 0,
            command: "nvim".to_string(),
            path: "/home/user".to_string(),
        });

        // Default: non-agent panes hidden
        assert!(state.hide_non_agent_panes);
        let nav = state.build_nav_items();
        // Should have Session + Agent, no NonAgentPane
        assert_eq!(nav.len(), 2);
        assert_eq!(nav[0], NavItem::Session("main".to_string()));
        assert_eq!(nav[1], NavItem::Agent(0));

        // Toggle to show non-agent panes
        state.hide_non_agent_panes = false;
        let nav = state.build_nav_items();
        assert_eq!(nav.len(), 3);
        assert_eq!(nav[2], NavItem::NonAgentPane(0));
    }

    #[test]
    fn test_hide_non_agent_sessions() {
        let mut state = AppState::new();
        state
            .agents
            .root_agents
            .push(make_agent("dev", "dev:0.0", 0, 0));
        state.all_sessions = vec!["dev".to_string(), "scratch".to_string()];

        // Default: non-agent sessions hidden
        assert!(state.hide_non_agent_sessions);
        let nav = state.build_nav_items();
        // Should only show "dev" session (has agents), not "scratch"
        assert_eq!(nav.len(), 2);
        assert_eq!(nav[0], NavItem::Session("dev".to_string()));
        assert_eq!(nav[1], NavItem::Agent(0));

        // Toggle to show all sessions
        state.hide_non_agent_sessions = false;
        let nav = state.build_nav_items();
        assert_eq!(nav.len(), 3); // dev session + agent + scratch session
        assert!(nav.contains(&NavItem::Session("scratch".to_string())));
    }

    #[test]
    fn test_search_jumps_to_match() {
        let mut state = AppState::new();
        state.hide_non_agent_sessions = false;
        state.hide_non_agent_panes = false;
        state
            .agents
            .root_agents
            .push(make_agent("alpha", "alpha:0.0", 0, 0));
        state
            .agents
            .root_agents
            .push(make_agent("beta", "beta:0.0", 0, 0));

        // Start on first agent
        state.cursor = TreeCursor::Agent(0);

        // Search for "beta" should jump cursor to beta session or agent
        state.search_query = Some("beta".to_string());
        state.apply_search();

        // Cursor should now be on something in beta
        let session = state.selected_session().unwrap().to_string();
        assert_eq!(session, "beta");
    }

    #[test]
    fn test_search_next_wraps() {
        let mut state = AppState::new();
        state.hide_non_agent_sessions = false;
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.0", 0, 0));
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.1", 0, 1));

        // Both agents are in "main", search for "main"
        state.search_query = Some("main".to_string());
        state.apply_search();
        // Should be on session header "main"
        assert_eq!(state.cursor, TreeCursor::Session("main".to_string()));

        // Next match should move to agent 0
        state.search_next();
        assert_eq!(state.cursor, TreeCursor::Agent(0));

        // Next match should move to agent 1
        state.search_next();
        assert_eq!(state.cursor, TreeCursor::Agent(1));
    }

    #[test]
    fn test_search_case_insensitive() {
        let mut state = AppState::new();
        state.hide_non_agent_sessions = false;
        state
            .agents
            .root_agents
            .push(make_agent("MyProject", "MyProject:0.0", 0, 0));

        state.search_query = Some("myproject".to_string());
        assert!(state.matches_search(&NavItem::Session("MyProject".to_string())));
    }

    #[test]
    fn test_search_cancel_restores_cursor() {
        let mut state = AppState::new();
        state.hide_non_agent_sessions = false;
        state
            .agents
            .root_agents
            .push(make_agent("alpha", "alpha:0.0", 0, 0));
        state
            .agents
            .root_agents
            .push(make_agent("beta", "beta:0.0", 0, 0));

        // Start on alpha agent
        state.cursor = TreeCursor::Agent(0);
        let original_cursor = state.cursor.clone();

        // Simulate search start: save cursor
        state.pre_search_cursor = Some(state.cursor.clone());
        state.search_query = Some("beta".to_string());
        state.apply_search();

        // Cursor should have moved away from original
        assert_ne!(state.cursor, original_cursor);

        // Cancel search: cursor should restore
        state.search_query = None;
        if let Some(cursor) = state.pre_search_cursor.take() {
            state.cursor = cursor;
        }
        assert_eq!(state.cursor, original_cursor);
    }

    #[test]
    fn test_search_prev_wraps_around() {
        let mut state = AppState::new();
        state.hide_non_agent_sessions = false;
        state
            .agents
            .root_agents
            .push(make_agent("alpha", "alpha:0.0", 0, 0));
        state
            .agents
            .root_agents
            .push(make_agent("beta", "beta:0.0", 0, 0));

        // Search matches both sessions. Start cursor at the FIRST match.
        state.search_query = Some("a".to_string());
        state.apply_search();
        let first_match = state.cursor.clone();

        // Prev from the first match should wrap to the LAST match
        state.search_prev();
        let wrapped = state.cursor.clone();
        assert_ne!(wrapped, first_match, "prev should wrap to a different item");

        // Going next from the wrapped position should return to the first match
        state.search_next();
        assert_eq!(state.cursor, first_match, "next should unwrap back");
    }

    #[test]
    fn test_search_confirm_on_session_expands_and_moves() {
        let mut state = AppState::new();
        state.hide_non_agent_sessions = false;
        state
            .agents
            .root_agents
            .push(make_agent("dev", "dev:0.0", 0, 0));

        // Collapse the session
        state.collapsed_sessions.insert("dev".to_string());

        // Search for "dev" — cursor lands on session header
        state.search_query = Some("dev".to_string());
        state.apply_search();
        assert_eq!(state.cursor, TreeCursor::Session("dev".to_string()));

        // Simulate what SearchConfirm does on a session:
        // expand and move to first child
        state.search_query = None;
        state.collapsed_sessions.remove("dev");
        state.select_next();

        // Should now be on the agent, not the session
        assert_eq!(state.cursor, TreeCursor::Agent(0));
        assert!(!state.collapsed_sessions.contains("dev"));
    }

    #[test]
    fn test_search_filters_nav_items() {
        let mut state = AppState::new();
        state.hide_non_agent_sessions = false;
        state
            .agents
            .root_agents
            .push(make_agent("alpha", "alpha:0.0", 0, 0));
        state
            .agents
            .root_agents
            .push(make_agent("beta", "beta:0.0", 0, 0));

        // Without search, both sessions visible
        let nav = state.build_nav_items();
        assert_eq!(nav.len(), 4); // 2 sessions + 2 agents

        // With search for "beta", only beta session and agent visible
        state.search_query = Some("beta".to_string());
        let nav = state.build_nav_items();
        assert_eq!(nav.len(), 2);
        assert_eq!(nav[0], NavItem::Session("beta".to_string()));
        assert_eq!(nav[1], NavItem::Agent(1));
    }

    #[test]
    fn test_search_empty_query_shows_all() {
        let mut state = AppState::new();
        state.hide_non_agent_sessions = false;
        state
            .agents
            .root_agents
            .push(make_agent("main", "main:0.0", 0, 0));

        // Empty search query should not filter
        state.search_query = Some(String::new());
        let nav = state.build_nav_items();
        assert_eq!(nav.len(), 2); // session + agent
    }

    #[test]
    fn test_flash_labels_single_char() {
        let labels = generate_flash_labels(5);
        assert_eq!(labels.len(), 5);
        // All should be single characters
        assert!(labels.iter().all(|l| l.len() == 1));
        // First label should be 'a' (home row priority)
        assert_eq!(labels[0], "a");
    }

    #[test]
    fn test_flash_labels_overflow_to_two_char() {
        let labels = generate_flash_labels(20);
        assert_eq!(labels.len(), 20);
        // First 14 are single char, rest are two-char with prefix
        assert_eq!(labels[13].len(), 1); // last single char: 'o'
        assert_eq!(labels[14].len(), 2); // first two-char
        assert!(labels[14].starts_with(';'));
    }

    #[test]
    fn test_focus_panel_methods() {
        let mut state = AppState::new();
        assert!(state.is_sidebar_focused()); // default

        state.focus_input();
        assert!(state.is_input_focused());
        assert!(!state.is_sidebar_focused());
        assert!(!state.is_preview_focused());

        state.focus_preview();
        assert!(state.is_preview_focused());
        assert!(!state.is_sidebar_focused());
        assert!(!state.is_input_focused());

        state.focus_sidebar();
        assert!(state.is_sidebar_focused());
    }

    #[test]
    fn test_sort_mode_cycling() {
        let mode = SortMode::Activity;
        assert_eq!(mode.next(), SortMode::Status);
        assert_eq!(mode.next().next(), SortMode::Activity);
    }

    #[test]
    fn test_sort_mode_label() {
        assert_eq!(SortMode::Activity.label(), "Recent");
        assert_eq!(SortMode::Status.label(), "Status");
    }

    #[test]
    fn test_cycle_sort_mode_on_state() {
        let mut state = AppState::new();
        assert_eq!(state.sort_mode, SortMode::Activity);
        state.cycle_sort_mode();
        assert_eq!(state.sort_mode, SortMode::Status);
        state.cycle_sort_mode();
        assert_eq!(state.sort_mode, SortMode::Activity);
    }

    #[test]
    fn test_sort_by_status_ordering() {
        use crate::agents::{AgentStatus, ApprovalType};

        let mut idle_agent = make_agent("idle_sess", "idle_sess:0.0", 0, 0);
        idle_agent.status = AgentStatus::Idle;

        let mut processing_agent = make_agent("proc_sess", "proc_sess:0.0", 0, 0);
        processing_agent.status = AgentStatus::Processing {
            activity: "working".to_string(),
        };

        let mut awaiting_agent = make_agent("await_sess", "await_sess:0.0", 0, 0);
        awaiting_agent.status = AgentStatus::AwaitingApproval {
            approval_type: ApprovalType::FileEdit,
            details: "test".to_string(),
        };

        let mut agents = vec![idle_agent, processing_agent, awaiting_agent];
        sort_agents(&mut agents, SortMode::Status);

        // Processing (priority 0) should come first, then AwaitingApproval (1), then Idle (4)
        assert!(matches!(agents[0].status, AgentStatus::Processing { .. }));
        assert!(matches!(
            agents[1].status,
            AgentStatus::AwaitingApproval { .. }
        ));
        assert!(matches!(agents[2].status, AgentStatus::Idle));
    }

    #[test]
    fn test_sort_by_activity_ordering() {
        let mut old_agent = make_agent("old_sess", "old_sess:0.0", 0, 0);
        // Manually set last_updated to an earlier time by subtracting duration
        old_agent.last_updated = Instant::now() - std::time::Duration::from_secs(100);

        let mut recent_agent = make_agent("recent_sess", "recent_sess:0.0", 0, 0);
        recent_agent.last_updated = Instant::now();

        let mut agents = vec![old_agent, recent_agent];
        sort_agents(&mut agents, SortMode::Activity);

        // Most recent session should come first
        assert_eq!(agents[0].session, "recent_sess");
        assert_eq!(agents[1].session, "old_sess");
    }

    #[test]
    fn test_build_nav_items_preserves_sort_order() {
        let mut state = AppState::new();

        // Add agents in specific order: beta first, then alpha
        let mut beta_agent = make_agent("beta", "beta:0.0", 0, 0);
        beta_agent.last_updated = Instant::now();
        let mut alpha_agent = make_agent("alpha", "alpha:0.0", 0, 0);
        alpha_agent.last_updated = Instant::now() - std::time::Duration::from_secs(100);

        state.agents.root_agents.push(beta_agent);
        state.agents.root_agents.push(alpha_agent);

        // build_nav_items should preserve order: beta session first, then alpha
        let nav = state.build_nav_items();
        assert_eq!(nav[0], NavItem::Session("beta".to_string()));
        assert_eq!(nav[1], NavItem::Agent(0)); // beta agent at index 0
        assert_eq!(nav[2], NavItem::Session("alpha".to_string()));
        assert_eq!(nav[3], NavItem::Agent(1)); // alpha agent at index 1
    }
}
