use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
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
    /// How to cycle through sounds when using a directory: "random" or "sequential"
    #[serde(default = "default_cycle")]
    pub cycle: String,
}

fn default_completed_sound() -> String {
    "Tink".to_string()
}

fn default_needs_input_sound() -> String {
    "Ping".to_string()
}

fn default_cycle() -> String {
    "random".to_string()
}

impl Default for NotificationSounds {
    fn default() -> Self {
        Self {
            completed: default_completed_sound(),
            needs_input: default_needs_input_sound(),
            cycle: default_cycle(),
        }
    }
}

/// How to pick the next sound from a directory
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CycleMode {
    Random,
    Sequential,
}

/// A resolved sound source — either a system sound name or a directory of files
#[derive(Debug, Clone)]
enum SoundSource {
    /// macOS system sound name (e.g. "Tink")
    SystemSound(String),
    /// Directory of audio files to cycle through
    Directory { files: Vec<PathBuf>, index: usize },
    /// No sound
    None,
}

/// What to actually play for a single notification
#[derive(Debug, Clone)]
enum ResolvedSound {
    /// Pass this name to --sound / sound name
    SystemSound(String),
    /// Play this file via afplay
    File(PathBuf),
    /// Silent
    None,
}

/// Audio file extensions supported by afplay on macOS
const SOUND_EXTENSIONS: &[&str] = &["aiff", "aif", "caf", "wav", "mp3"];

impl SoundSource {
    /// Parse a config string into a sound source.
    /// If it looks like a path and resolves to a directory, scan it for audio files.
    /// If it's "none", return None. Otherwise treat as a system sound name.
    fn from_config(value: &str) -> Self {
        if value.eq_ignore_ascii_case("none") || value.is_empty() {
            return Self::None;
        }

        // Check if it looks like a path
        let expanded = if let Some(rest) = value.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                home.join(rest)
            } else {
                PathBuf::from(value)
            }
        } else if value == "~" {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from(value))
        } else if value.starts_with('/') {
            PathBuf::from(value)
        } else if value.starts_with('.') || value.contains('/') {
            // Relative path — resolve relative to config dir (~/.config/tmuxcc/)
            if let Some(config_dir) = dirs::config_dir() {
                config_dir.join("tmuxcc").join(value)
            } else {
                PathBuf::from(value)
            }
        } else {
            PathBuf::from(value)
        };

        if expanded.is_dir() {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&expanded)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| SOUND_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
                })
                .collect();
            files.sort();

            if files.is_empty() {
                tracing::warn!(dir = %expanded.display(), "sound directory is empty or has no audio files");
                return Self::None;
            }

            tracing::info!(dir = %expanded.display(), count = files.len(), "loaded sound directory");
            Self::Directory { files, index: 0 }
        } else if value.starts_with('/') || value.starts_with('~') || value.starts_with('.') {
            // Looks like a path but isn't a directory
            tracing::warn!(path = %value, "sound path is not a directory, treating as system sound name");
            Self::SystemSound(value.to_string())
        } else {
            Self::SystemSound(value.to_string())
        }
    }

    /// Pick the next sound to play
    fn resolve(&mut self, cycle_mode: CycleMode) -> ResolvedSound {
        match self {
            Self::SystemSound(name) => ResolvedSound::SystemSound(name.clone()),
            Self::Directory { files, index } => {
                if files.is_empty() {
                    return ResolvedSound::None;
                }
                let pick = match cycle_mode {
                    CycleMode::Sequential => {
                        let i = *index;
                        *index = (*index + 1) % files.len();
                        i
                    }
                    CycleMode::Random => {
                        // Cheap pseudo-random without adding a dep.
                        // Use system time nanos (high entropy) mixed with a counter
                        // to avoid repeats on rapid calls.
                        let nanos = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.subsec_nanos() as usize)
                            .unwrap_or(0);
                        let pick =
                            nanos.wrapping_add(*index).wrapping_mul(2654435761) % files.len();
                        *index = index.wrapping_add(1);
                        pick
                    }
                };
                ResolvedSound::File(files[pick].clone())
            }
            Self::None => ResolvedSound::None,
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
    /// Active notification sound profile name
    #[serde(default = "default_active_profile")]
    pub active_profile: String,
    /// Named sound profiles
    #[serde(default = "default_profiles")]
    pub profiles: HashMap<String, NotificationSounds>,
    #[serde(default)]
    pub backend: NotificationBackendConfig,
    /// Legacy: flat sounds config for backward compat
    #[serde(default, skip_serializing)]
    sounds: Option<NotificationSounds>,
}

fn default_enabled() -> bool {
    true
}

fn default_cooldown_secs() -> u64 {
    10
}

fn default_active_profile() -> String {
    "default".to_string()
}

fn default_profiles() -> HashMap<String, NotificationSounds> {
    let mut m = HashMap::new();
    m.insert("default".to_string(), NotificationSounds::default());
    m
}

impl NotificationConfig {
    /// Resolve the active sound profile. Falls back to "default", then to NotificationSounds::default().
    pub fn resolve_sounds(&self) -> &NotificationSounds {
        // First try the active profile
        if let Some(profile) = self.profiles.get(&self.active_profile) {
            return profile;
        }
        // Fall back to "default" profile
        if let Some(profile) = self.profiles.get("default") {
            return profile;
        }
        // This shouldn't happen since default_profiles always has "default",
        // but return a static fallback for safety
        static FALLBACK: std::sync::LazyLock<NotificationSounds> =
            std::sync::LazyLock::new(NotificationSounds::default);
        &FALLBACK
    }

    /// Return the names of all available profiles
    pub fn profile_names(&self) -> Vec<&str> {
        self.profiles.keys().map(|k| k.as_str()).collect()
    }

    /// Cycle the active profile to the next one (alphabetical order)
    pub fn cycle_profile(&mut self) {
        let mut names: Vec<&String> = self.profiles.keys().collect();
        names.sort();
        if names.is_empty() {
            return;
        }
        let current_idx = names.iter().position(|n| **n == self.active_profile);
        let next_idx = match current_idx {
            Some(i) => (i + 1) % names.len(),
            None => 0,
        };
        self.active_profile = names[next_idx].clone();
    }

    /// Post-deserialization migration: if legacy `sounds` field is present and
    /// profiles only has the default entry, use sounds as the "default" profile.
    pub fn migrate_legacy(&mut self) {
        if let Some(sounds) = self.sounds.take() {
            // Only migrate if profiles is exactly the default (one "default" entry)
            if self.profiles.len() <= 1 {
                self.profiles.insert("default".to_string(), sounds);
            }
        }
    }
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            cooldown_secs: default_cooldown_secs(),
            active_profile: default_active_profile(),
            profiles: default_profiles(),
            backend: NotificationBackendConfig::default(),
            sounds: None,
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
    completed_sound: SoundSource,
    needs_input_sound: SoundSource,
    cycle_mode: CycleMode,
    backend: NotificationBackend,
}

impl Notifier {
    pub fn new(config: &NotificationConfig) -> Self {
        let backend = detect_backend(&config.backend);
        tracing::info!(%backend, "notification backend selected");

        let sounds = config.resolve_sounds();
        let cycle_mode = if sounds.cycle.eq_ignore_ascii_case("sequential") {
            CycleMode::Sequential
        } else {
            CycleMode::Random
        };

        tracing::info!(
            active_profile = %config.active_profile,
            completed = %sounds.completed,
            needs_input = %sounds.needs_input,
            cycle = %sounds.cycle,
            "notification sound profile loaded"
        );

        Self {
            last_notified: HashMap::new(),
            cooldown: Duration::from_secs(config.cooldown_secs),
            enabled: config.enabled,
            app_focused: false,
            completed_sound: SoundSource::from_config(&sounds.completed),
            needs_input_sound: SoundSource::from_config(&sounds.needs_input),
            cycle_mode,
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

        let event_detail = match (&event, new) {
            (
                NotificationEvent::NeedsInput,
                AgentStatus::AwaitingApproval {
                    approval_type,
                    details,
                },
            ) => {
                if details.is_empty() {
                    format!("{}", approval_type)
                } else {
                    let d = if details.chars().count() > 60 {
                        let truncated: String = details.chars().take(57).collect();
                        format!("{}...", truncated)
                    } else {
                        details.clone()
                    };
                    format!("{}: {}", approval_type.short_desc(), d)
                }
            }
            (NotificationEvent::Completed, _) => "Finished".to_string(),
            _ => return,
        };

        // Resolve sound for this event
        let resolved_sound = match event {
            NotificationEvent::Completed => self.completed_sound.resolve(self.cycle_mode),
            NotificationEvent::NeedsInput => self.needs_input_sound.resolve(self.cycle_mode),
        };

        let body = format!("{} — {}", pane_location, event_detail);

        tracing::info!(
            agent = %info.agent_id,
            event = ?event,
            backend = %self.backend,
            body = %body,
            sound = ?resolved_sound,
            "firing notification"
        );

        send_notification(
            &self.backend,
            "tmuxcc",
            &info.agent_label,
            &body,
            &resolved_sound,
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
    sound: &ResolvedSound,
    group: &str,
    pane_target: &str,
) {
    // For file-based sounds, play via afplay separately
    if let ResolvedSound::File(path) = sound {
        let path = path.clone();
        tokio::spawn(async move {
            let result = tokio::process::Command::new("afplay")
                .arg(&path)
                .output()
                .await;
            if let Err(e) = result {
                tracing::warn!(path = %path.display(), "failed to play sound file: {}", e);
            }
        });
    }

    // Extract system sound name (or empty for file/none)
    let system_sound = match sound {
        ResolvedSound::SystemSound(name) => name.as_str(),
        _ => "",
    };

    match backend {
        NotificationBackend::Alerter => {
            send_alerter(title, subtitle, body, system_sound, group, pane_target)
        }
        NotificationBackend::Osascript => send_osascript(title, subtitle, body, system_sound),
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
            completed_sound: SoundSource::None,
            needs_input_sound: SoundSource::None,
            cycle_mode: CycleMode::Random,
            backend: NotificationBackend::Bel,
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
            completed_sound: SoundSource::None,
            needs_input_sound: SoundSource::None,
            cycle_mode: CycleMode::Random,
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
    fn test_sound_source_system_sound() {
        let source = SoundSource::from_config("Tink");
        assert!(matches!(source, SoundSource::SystemSound(ref s) if s == "Tink"));
    }

    #[test]
    fn test_sound_source_none() {
        assert!(matches!(
            SoundSource::from_config("none"),
            SoundSource::None
        ));
        assert!(matches!(
            SoundSource::from_config("NONE"),
            SoundSource::None
        ));
        assert!(matches!(SoundSource::from_config(""), SoundSource::None));
    }

    #[test]
    fn test_sound_source_directory() {
        let dir = std::env::temp_dir().join("tmuxcc_test_sounds");
        let _ = std::fs::create_dir_all(&dir);
        // Create fake audio files
        std::fs::write(dir.join("01-clip.aiff"), b"fake").unwrap();
        std::fs::write(dir.join("02-clip.wav"), b"fake").unwrap();
        std::fs::write(dir.join("not-audio.txt"), b"fake").unwrap();

        let source = SoundSource::from_config(dir.to_str().unwrap());
        match source {
            SoundSource::Directory { files, .. } => {
                assert_eq!(files.len(), 2);
                // Should be sorted
                assert!(files[0]
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .contains("01"));
                assert!(files[1]
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .contains("02"));
            }
            _ => panic!("expected Directory, got {:?}", source),
        }

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sound_resolve_sequential() {
        let dir = std::env::temp_dir().join("tmuxcc_test_seq");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("a.aiff"), b"fake").unwrap();
        std::fs::write(dir.join("b.aiff"), b"fake").unwrap();
        std::fs::write(dir.join("c.aiff"), b"fake").unwrap();

        let mut source = SoundSource::from_config(dir.to_str().unwrap());

        // Sequential should cycle a -> b -> c -> a
        let r1 = source.resolve(CycleMode::Sequential);
        let r2 = source.resolve(CycleMode::Sequential);
        let r3 = source.resolve(CycleMode::Sequential);
        let r4 = source.resolve(CycleMode::Sequential);

        let name = |r: &ResolvedSound| match r {
            ResolvedSound::File(p) => p.file_name().unwrap().to_str().unwrap().to_string(),
            _ => panic!("expected File"),
        };

        assert_eq!(name(&r1), "a.aiff");
        assert_eq!(name(&r2), "b.aiff");
        assert_eq!(name(&r3), "c.aiff");
        assert_eq!(name(&r4), "a.aiff"); // wraps around

        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn test_profiles_default() {
        let config = NotificationConfig::default();
        assert_eq!(config.active_profile, "default");
        assert!(config.profiles.contains_key("default"));
        let sounds = config.resolve_sounds();
        assert_eq!(sounds.completed, "Tink");
        assert_eq!(sounds.needs_input, "Ping");
    }

    #[test]
    fn test_profiles_active_selection() {
        let mut config = NotificationConfig::default();
        config.profiles.insert(
            "custom".to_string(),
            NotificationSounds {
                completed: "Glass".to_string(),
                needs_input: "Submarine".to_string(),
                cycle: "sequential".to_string(),
            },
        );
        config.active_profile = "custom".to_string();
        let sounds = config.resolve_sounds();
        assert_eq!(sounds.completed, "Glass");
        assert_eq!(sounds.needs_input, "Submarine");
        assert_eq!(sounds.cycle, "sequential");
    }

    #[test]
    fn test_profiles_unknown_fallback() {
        let config = NotificationConfig {
            active_profile: "nonexistent".to_string(),
            ..NotificationConfig::default()
        };
        // Should fall back to "default" profile
        let sounds = config.resolve_sounds();
        assert_eq!(sounds.completed, "Tink");
        assert_eq!(sounds.needs_input, "Ping");
    }

    #[test]
    fn test_profiles_backward_compat() {
        // Simulate old config with [notifications.sounds] section
        let toml_str = r#"
enabled = true
cooldown_secs = 10

[sounds]
completed = "Glass"
needs_input = "Submarine"
cycle = "sequential"
"#;
        let mut config: NotificationConfig = toml::from_str(toml_str).unwrap();
        config.migrate_legacy();

        // The legacy sounds should have been migrated into the "default" profile
        let sounds = config.resolve_sounds();
        assert_eq!(sounds.completed, "Glass");
        assert_eq!(sounds.needs_input, "Submarine");
        assert_eq!(sounds.cycle, "sequential");
    }

    #[test]
    fn test_profiles_toml_roundtrip() {
        let mut config = NotificationConfig::default();
        config.profiles.insert(
            "itysl".to_string(),
            NotificationSounds {
                completed: "~/.config/tmuxcc/sounds/completed/".to_string(),
                needs_input: "~/.config/tmuxcc/sounds/needs_input/".to_string(),
                cycle: "random".to_string(),
            },
        );
        config.active_profile = "itysl".to_string();

        let serialized = toml::to_string(&config).unwrap();
        let deserialized: NotificationConfig = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.active_profile, "itysl");
        assert!(deserialized.profiles.contains_key("default"));
        assert!(deserialized.profiles.contains_key("itysl"));
        let sounds = deserialized.resolve_sounds();
        assert_eq!(
            sounds.completed,
            "~/.config/tmuxcc/sounds/completed/"
        );
    }

    #[test]
    fn test_profiles_cycle() {
        let mut config = NotificationConfig::default();
        config.profiles.insert(
            "alt".to_string(),
            NotificationSounds::default(),
        );
        // Start on "default"
        assert_eq!(config.active_profile, "default");
        config.cycle_profile();
        // After cycling, should move to next alphabetically
        // "alt" < "default", so sorted order is ["alt", "default"]
        // From "default" (index 1) -> wraps to "alt" (index 0)
        assert_eq!(config.active_profile, "alt");
        config.cycle_profile();
        assert_eq!(config.active_profile, "default");
    }
}
