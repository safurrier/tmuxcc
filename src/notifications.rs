use std::collections::HashMap;
use std::io::Write;
use std::time::{Duration, Instant};

use crate::agents::AgentStatus;

/// Notification event type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEvent {
    Completed,
    NeedsInput,
}

/// Sound configuration for notifications
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NotificationSounds {
    #[serde(default = "default_completed_sound")]
    pub completed: String,
    #[serde(default = "default_needs_input_sound")]
    pub needs_input: String,
}

fn default_completed_sound() -> String {
    "Tink".to_string()
}

fn default_needs_input_sound() -> String {
    "Ping".to_string()
}

impl Default for NotificationSounds {
    fn default() -> Self {
        Self {
            completed: default_completed_sound(),
            needs_input: default_needs_input_sound(),
        }
    }
}

/// Available notification backends
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationBackendConfig {
    #[default]
    Auto,
    Alerter,
    Osascript,
    #[serde(rename = "notify-send")]
    NotifySend,
    Bel,
}

/// Resolved notification backend
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationBackend {
    Alerter,
    Osascript,
    NotifySend,
    Bel,
}

impl std::fmt::Display for NotificationBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alerter => write!(f, "alerter"),
            Self::Osascript => write!(f, "osascript"),
            Self::NotifySend => write!(f, "notify-send"),
            Self::Bel => write!(f, "BEL"),
        }
    }
}

/// Notification configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NotificationConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
    #[serde(default)]
    pub sounds: NotificationSounds,
    #[serde(default)]
    pub backend: NotificationBackendConfig,
}

fn default_enabled() -> bool {
    true
}

fn default_cooldown_secs() -> u64 {
    10
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            cooldown_secs: default_cooldown_secs(),
            sounds: NotificationSounds::default(),
            backend: NotificationBackendConfig::default(),
        }
    }
}

/// Info about the agent pane, passed into the notifier for richer content
#[derive(Debug, Clone)]
pub struct AgentNotificationInfo {
    pub agent_id: String,
    pub agent_label: String,
    pub session: String,
    pub window: u32,
    pub window_name: String,
    pub target: String,
    pub is_active_pane: bool,
}

/// Manages desktop notifications for agent status transitions
pub struct Notifier {
    last_notified: HashMap<String, Instant>,
    cooldown: Duration,
    pub enabled: bool,
    pub app_focused: bool,
    sounds: NotificationSounds,
    backend: NotificationBackend,
}

impl Notifier {
    pub fn new(config: &NotificationConfig) -> Self {
        let backend = detect_backend(&config.backend);
        tracing::info!(%backend, "notification backend selected");

        Self {
            last_notified: HashMap::new(),
            cooldown: Duration::from_secs(config.cooldown_secs),
            enabled: config.enabled,
            app_focused: false,
            sounds: config.sounds.clone(),
            backend,
        }
    }

    /// Check a status transition and fire a notification if warranted.
    pub fn check_and_notify(
        &mut self,
        info: &AgentNotificationInfo,
        old: &AgentStatus,
        new: &AgentStatus,
        is_selected_in_ui: bool,
    ) {
        if !self.enabled {
            tracing::debug!(agent = %info.agent_id, "notification suppressed: disabled");
            return;
        }

        tracing::debug!(
            agent = %info.agent_id,
            old = ?old,
            new = ?new,
            is_active_pane = info.is_active_pane,
            is_selected = is_selected_in_ui,
            app_focused = self.app_focused,
            "check_and_notify called"
        );

        // Suppress if user is looking at this pane in tmux
        // (active pane + active window + session has an attached client)
        if info.is_active_pane {
            tracing::debug!(agent = %info.agent_id, "notification suppressed: user viewing pane");
            return;
        }

        // Suppress if user is looking at this agent in tmuxcc
        if self.app_focused && is_selected_in_ui {
            tracing::debug!(agent = %info.agent_id, "notification suppressed: focused+selected");
            return;
        }

        // Determine notification event from transition
        let event = match (old, new) {
            (AgentStatus::Processing { .. }, AgentStatus::Idle) => {
                Some(NotificationEvent::Completed)
            }
            (prev, AgentStatus::AwaitingApproval { .. })
                if !matches!(prev, AgentStatus::AwaitingApproval { .. }) =>
            {
                Some(NotificationEvent::NeedsInput)
            }
            _ => None,
        };

        let event = match event {
            Some(e) => e,
            None => {
                tracing::debug!(agent = %info.agent_id, "no notification: no relevant transition");
                return;
            }
        };

        // Cooldown check
        if let Some(last) = self.last_notified.get(&info.agent_id) {
            if last.elapsed() < self.cooldown {
                tracing::debug!(agent = %info.agent_id, "notification suppressed: cooldown");
                return;
            }
        }

        self.last_notified
            .insert(info.agent_id.clone(), Instant::now());

        // Build notification content
        let pane_location = format!("{}:{} \"{}\"", info.session, info.window, info.window_name);

        let (event_detail, sound) = match (&event, new) {
            (
                NotificationEvent::NeedsInput,
                AgentStatus::AwaitingApproval {
                    approval_type,
                    details,
                },
            ) => {
                let detail = if details.is_empty() {
                    format!("{}", approval_type)
                } else {
                    let d = if details.chars().count() > 60 {
                        let truncated: String = details.chars().take(57).collect();
                        format!("{}...", truncated)
                    } else {
                        details.clone()
                    };
                    format!("{}: {}", approval_type.short_desc(), d)
                };
                (detail, &self.sounds.needs_input)
            }
            (NotificationEvent::Completed, _) => ("Finished".to_string(), &self.sounds.completed),
            _ => return,
        };

        let body = format!("{} — {}", pane_location, event_detail);

        tracing::info!(
            agent = %info.agent_id,
            event = ?event,
            backend = %self.backend,
            body = %body,
            "firing notification"
        );

        send_notification(
            &self.backend,
            "tmuxcc",
            &info.agent_label,
            &body,
            sound,
            &info.agent_id,
            &info.target,
        );
    }

    /// Returns the name of the active backend
    pub fn backend_name(&self) -> &str {
        match self.backend {
            NotificationBackend::Alerter => "alerter",
            NotificationBackend::Osascript => "osascript",
            NotificationBackend::NotifySend => "notify-send",
            NotificationBackend::Bel => "bel",
        }
    }
}

/// Detect the best available notification backend
fn detect_backend(config: &NotificationBackendConfig) -> NotificationBackend {
    match config {
        NotificationBackendConfig::Alerter => NotificationBackend::Alerter,
        NotificationBackendConfig::Osascript => NotificationBackend::Osascript,
        NotificationBackendConfig::NotifySend => NotificationBackend::NotifySend,
        NotificationBackendConfig::Bel => NotificationBackend::Bel,
        NotificationBackendConfig::Auto => {
            if command_exists("alerter") {
                NotificationBackend::Alerter
            } else if command_exists("osascript") {
                NotificationBackend::Osascript
            } else if command_exists("notify-send") {
                NotificationBackend::NotifySend
            } else {
                NotificationBackend::Bel
            }
        }
    }
}

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Dispatch notification to the appropriate backend
fn send_notification(
    backend: &NotificationBackend,
    title: &str,
    subtitle: &str,
    body: &str,
    sound: &str,
    group: &str,
    pane_target: &str,
) {
    match backend {
        NotificationBackend::Alerter => {
            send_alerter(title, subtitle, body, sound, group, pane_target)
        }
        NotificationBackend::Osascript => send_osascript(title, subtitle, body, sound),
        NotificationBackend::NotifySend => send_notify_send(title, subtitle, body),
        NotificationBackend::Bel => send_bel(),
    }
}

/// Send notification via alerter with click-to-jump support
fn send_alerter(
    title: &str,
    subtitle: &str,
    body: &str,
    sound: &str,
    group: &str,
    pane_target: &str,
) {
    let title = title.to_string();
    let subtitle = subtitle.to_string();
    let body = body.to_string();
    let sound = sound.to_string();
    let group = format!("tmuxcc:{}", group);
    let pane_target = pane_target.to_string();

    // Extract window target (session:window) from full target (session:window.pane)
    let window_target = if let Some(pos) = pane_target.rfind('.') {
        pane_target[..pos].to_string()
    } else {
        pane_target.clone()
    };

    tokio::spawn(async move {
        let mut args = vec![
            "--title".to_string(),
            title,
            "--subtitle".to_string(),
            subtitle,
            "--message".to_string(),
            body,
            "--group".to_string(),
            group,
            "--timeout".to_string(),
            "10".to_string(),
            "--actions".to_string(),
            "Go to pane".to_string(),
            "--close-label".to_string(),
            "Dismiss".to_string(),
        ];

        if sound.to_lowercase() != "none" && !sound.is_empty() {
            args.push("--sound".to_string());
            args.push(sound);
        }

        tracing::debug!(args = ?args, "spawning alerter");
        let result = tokio::process::Command::new("alerter")
            .args(&args)
            .output()
            .await;

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                tracing::info!(
                    exit_code = output.status.code(),
                    stdout = %stdout,
                    stderr = %stderr,
                    "alerter process completed"
                );
                if stdout == "Go to pane"
                    || stdout == "@ACTIONCLICKED"
                    || stdout == "@CONTENTCLICKED"
                {
                    tracing::info!(pane = %pane_target, "notification click: jumping to pane");
                    let _ = std::process::Command::new("tmux")
                        .args(["select-window", "-t", &window_target])
                        .output();
                    let _ = std::process::Command::new("tmux")
                        .args(["select-pane", "-t", &pane_target])
                        .output();
                }
            }
            Err(e) => {
                tracing::warn!("failed to send alerter notification: {}", e);
            }
        }
    });
}

/// Send notification via osascript (macOS fallback)
fn send_osascript(title: &str, subtitle: &str, body: &str, sound: &str) {
    let title = title.to_string();
    let subtitle = subtitle.to_string();
    let body = body.to_string();
    let sound = sound.to_string();

    tokio::spawn(async move {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");

        let mut script = format!(
            "display notification \"{}\" with title \"{}\" subtitle \"{}\"",
            esc(&body),
            esc(&title),
            esc(&subtitle),
        );

        if sound.to_lowercase() != "none" && !sound.is_empty() {
            script.push_str(&format!(" sound name \"{}\"", esc(&sound)));
        }

        let result = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .await;

        if let Err(e) = result {
            tracing::warn!("failed to send osascript notification: {}", e);
        }
    });
}

/// Send notification via notify-send (Linux)
fn send_notify_send(title: &str, subtitle: &str, body: &str) {
    let title = title.to_string();
    let full_body = format!("{}\n{}", subtitle, body);

    tokio::spawn(async move {
        let result = tokio::process::Command::new("notify-send")
            .args([&title, &full_body, "--urgency=normal"])
            .output()
            .await;

        if let Err(e) = result {
            tracing::warn!("failed to send notify-send notification: {}", e);
        }
    });
}

/// Send BEL character to trigger terminal bell (works over SSH)
fn send_bel() {
    // Write BEL to stderr to avoid interfering with TUI on stdout
    let _ = std::io::stderr().write_all(b"\x07");
    let _ = std::io::stderr().flush();
    tracing::debug!("sent BEL notification");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::ApprovalType;

    fn make_notifier() -> Notifier {
        Notifier {
            last_notified: HashMap::new(),
            cooldown: Duration::from_secs(0),
            enabled: true,
            app_focused: false,
            sounds: NotificationSounds::default(),
            backend: NotificationBackend::Bel, // Use BEL in tests to avoid spawning processes
        }
    }

    fn make_info(agent_id: &str) -> AgentNotificationInfo {
        AgentNotificationInfo {
            agent_id: agent_id.to_string(),
            agent_label: "Claude · test".to_string(),
            session: "main".to_string(),
            window: 2,
            window_name: "claude-test".to_string(),
            target: "main:2.0".to_string(),
            is_active_pane: false,
        }
    }

    #[test]
    fn test_no_notification_on_same_status() {
        let mut n = make_notifier();
        let info = make_info("a1");
        let idle = AgentStatus::Idle;
        n.check_and_notify(&info, &idle, &idle, false);
        assert!(n.last_notified.is_empty());
    }

    #[test]
    fn test_no_notification_when_disabled() {
        let mut n = make_notifier();
        n.enabled = false;
        let info = make_info("a1");
        let old = AgentStatus::Processing {
            activity: "working".into(),
        };
        let new = AgentStatus::Idle;
        n.check_and_notify(&info, &old, &new, false);
        assert!(n.last_notified.is_empty());
    }

    #[test]
    fn test_no_notification_when_active_pane() {
        let mut n = make_notifier();
        let mut info = make_info("a1");
        info.is_active_pane = true;
        let old = AgentStatus::Processing {
            activity: "working".into(),
        };
        let new = AgentStatus::Idle;
        n.check_and_notify(&info, &old, &new, false);
        assert!(n.last_notified.is_empty());
    }

    #[test]
    fn test_no_notification_when_focused_and_selected() {
        let mut n = make_notifier();
        n.app_focused = true;
        let info = make_info("a1");
        let old = AgentStatus::Processing {
            activity: "working".into(),
        };
        let new = AgentStatus::Idle;
        n.check_and_notify(&info, &old, &new, true);
        assert!(n.last_notified.is_empty());
    }

    #[tokio::test]
    async fn test_notification_when_focused_but_not_selected() {
        let mut n = make_notifier();
        n.app_focused = true;
        let info = make_info("a1");
        let old = AgentStatus::Processing {
            activity: "working".into(),
        };
        let new = AgentStatus::Idle;
        n.check_and_notify(&info, &old, &new, false);
        assert!(n.last_notified.contains_key("a1"));
    }

    #[tokio::test]
    async fn test_cooldown_prevents_spam() {
        let mut n = Notifier {
            last_notified: HashMap::new(),
            cooldown: Duration::from_secs(60),
            enabled: true,
            app_focused: false,
            sounds: NotificationSounds::default(),
            backend: NotificationBackend::Bel,
        };
        let info = make_info("a1");
        let old = AgentStatus::Processing {
            activity: "working".into(),
        };
        let new = AgentStatus::Idle;

        n.check_and_notify(&info, &old, &new, false);
        assert!(n.last_notified.contains_key("a1"));

        let first_time = *n.last_notified.get("a1").unwrap();
        n.check_and_notify(&info, &old, &new, false);
        assert_eq!(*n.last_notified.get("a1").unwrap(), first_time);
    }

    #[tokio::test]
    async fn test_awaiting_approval_triggers_notification() {
        let mut n = make_notifier();
        let info = make_info("a1");
        let old = AgentStatus::Idle;
        let new = AgentStatus::AwaitingApproval {
            approval_type: ApprovalType::FileEdit,
            details: "src/main.rs".into(),
        };
        n.check_and_notify(&info, &old, &new, false);
        assert!(n.last_notified.contains_key("a1"));
    }

    #[test]
    fn test_idle_to_processing_no_notification() {
        let mut n = make_notifier();
        let info = make_info("a1");
        let old = AgentStatus::Idle;
        let new = AgentStatus::Processing {
            activity: "thinking".into(),
        };
        n.check_and_notify(&info, &old, &new, false);
        assert!(n.last_notified.is_empty());
    }

    #[test]
    fn test_approval_to_approval_no_notification() {
        let mut n = make_notifier();
        let info = make_info("a1");
        let old = AgentStatus::AwaitingApproval {
            approval_type: ApprovalType::FileEdit,
            details: "foo.rs".into(),
        };
        let new = AgentStatus::AwaitingApproval {
            approval_type: ApprovalType::ShellCommand,
            details: "cargo build".into(),
        };
        n.check_and_notify(&info, &old, &new, false);
        assert!(n.last_notified.is_empty());
    }

    #[test]
    fn test_backend_detection_auto() {
        // Auto should resolve to something (platform-dependent)
        let backend = detect_backend(&NotificationBackendConfig::Auto);
        // Just verify it doesn't panic and returns a valid variant
        assert!(matches!(
            backend,
            NotificationBackend::Alerter
                | NotificationBackend::Osascript
                | NotificationBackend::NotifySend
                | NotificationBackend::Bel
        ));
    }

    #[test]
    fn test_backend_detection_forced() {
        assert_eq!(
            detect_backend(&NotificationBackendConfig::Bel),
            NotificationBackend::Bel
        );
        assert_eq!(
            detect_backend(&NotificationBackendConfig::Osascript),
            NotificationBackend::Osascript
        );
    }
}
